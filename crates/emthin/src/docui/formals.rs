//! The `\formal` verification seam: ask `logos` what a CNL sentence reduces to,
//! and splice the answer beside the claim.
//!
//! # Why a subprocess
//!
//! The kernel surface is `logos unf <sentence> --json`
//! (`logos/src/cli/mod.rs::cmd_unf`), not an in-process call. `prob_kernel`
//! depends on `logos`, so `logos` cannot depend on it back, and the emthin
//! compositor already runs the `logos` crate as a *library* for types and
//! verification while treating the kernel as external. The subprocess is the
//! seam that respects that.
//!
//! # What this is not
//!
//! It is not required. A document with no `\formal` statements never invokes it,
//! and a missing `logos` binary is a **silent no-op** rather than a failure — the
//! same rule `australVM/lib/formalize_plugin.ml` follows, and for the same
//! reason: a checkout without the binary must still open the editor.
//!
//! What it is *not* silent about is a binary that ran and failed. Those are
//! reported as [`Verdict::Failed`] with the kernel's own reason, so an
//! out-of-lexicon word is named in the document rather than turning into a
//! silently-unverified block.
//!
//! # Cost, and the cache
//!
//! One subprocess per distinct sentence per run. Sentences are cached by their
//! text, so re-laying out the same document costs nothing, and two steps with
//! the same CNL are asked once — which is also how the pipeline's own identity
//! rule (two sentences denoting one term are one node) is respected at the
//! transport level: the second one gets the first one's answer, including its
//! UNF hash, and if two steps then collide the reader sees the same hash twice.

use mathed_core::formalize::{self, ResolvedFormal, Verdict};
use mathed_core::TransformOptions;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Env var naming the kernel binary, mirroring the plugin's `UNFER_LOGOS_BIN`.
pub const KERNEL_BIN_VAR: &str = "EMTHIN_LOGOS_BIN";

/// Default binary name, resolved on `$PATH`.
const DEFAULT_BIN: &str = "logos";

/// Where to find the kernel, and what it said last time.
///
/// Construction never fails and never probes: [`FormalVerifier::new`] reads the
/// environment, and the first [`FormalVerifier::verify`] decides whether the
/// binary is usable. Deferring the probe keeps construction cheap enough to do
/// on every compositor start.
pub struct FormalVerifier {
    binary: Option<PathBuf>,
    /// Sentence text -> verdict. The key is the sentence, not the step, so two
    /// steps with the same CNL cost one subprocess.
    cache: HashMap<String, Verdict>,
    /// Subprocesses actually run, for diagnostics.
    calls: usize,
}

impl Default for FormalVerifier {
    fn default() -> Self {
        Self::new()
    }
}

impl FormalVerifier {
    pub fn new() -> Self {
        Self::with_binary(resolve_binary(std::env::var_os(KERNEL_BIN_VAR)))
    }

    /// A verifier pinned to one binary, for tests and for a pinned deployment.
    pub fn with_binary(binary: Option<PathBuf>) -> Self {
        FormalVerifier {
            binary,
            cache: HashMap::new(),
            calls: 0,
        }
    }

    /// Whether a kernel binary was found. `None` makes every verdict a no-op.
    pub fn is_available(&self) -> bool {
        self.binary.is_some()
    }

    /// Pin a different kernel, or unpin it with `None`.
    ///
    /// Clearing the cache is the point: the cached verdicts came from the *old*
    /// binary, so keeping them would let one kernel's answers outlive it. A
    /// no-op swap to the same path is left alone, so re-pinning does not
    /// re-run every sentence.
    pub fn set_binary(&mut self, binary: Option<PathBuf>) {
        if self.binary == binary {
            return;
        }
        self.binary = binary;
        self.cache.clear();
    }

    /// The kernel path in use, if one was found.
    pub fn binary(&self) -> Option<&Path> {
        self.binary.as_deref()
    }

    /// How many subprocesses have actually run. Zero for a document with no
    /// `\formal` statements, which is the common case.
    pub fn calls(&self) -> usize {
        self.calls
    }

    /// The `\formal` steps in `doc_text`, in document order.
    pub fn steps(doc_text: &str) -> Vec<ResolvedFormal> {
        let scan = mathed_core::markers::scan(doc_text);
        let segments = mathed_core::markers::resolve_segments(&scan);
        formalize::formals_in_segments(&segments)
    }

    /// Ask the kernel about one sentence, caching by its text.
    ///
    /// An unavailable kernel yields `None`, which every caller treats as "leave
    /// the declaration alone" — see the module docs.
    pub fn verify(&mut self, sentence: &str) -> Option<Verdict> {
        if let Some(hit) = self.cache.get(sentence) {
            return Some(hit.clone());
        }
        let binary = self.binary.clone()?;
        self.calls += 1;
        let verdict = ask(&binary, sentence);
        self.cache.insert(sentence.to_string(), verdict.clone());
        Some(verdict)
    }

    /// The annotation splices for a document: verdict markup keyed by each
    /// step's caption **start**, which is what `TransformOptions::annotations`
    /// is keyed by and where `transform` inserts it.
    ///
    /// Empty when there is no kernel or no `\formal` steps, so the overwhelmingly
    /// common path costs one scan and nothing else.
    pub fn annotations(&mut self, doc_text: &str) -> HashMap<usize, String> {
        let mut out = HashMap::new();
        let Some(binary) = self.binary.clone() else {
            return out;
        };
        for step in Self::steps(doc_text) {
            let verdict = self
                .verify(&step.spec.cnl)
                .unwrap_or(Verdict::Failed("no kernel binary".into()));
            out.insert(step.span.start, formalize::verdict_markup(&verdict));
        }
        // Referenced so the unused-import warning cannot hide a real dependency
        // change in the binary path above.
        let _ = binary;
        out
    }

    /// Merge this verifier's annotations into a layout pass's options.
    ///
    /// The verdict is spliced *after* the declaration because `transform`'s
    /// documented priority puts content (the declared CNL) before results (the
    /// kernel's answer), and both land at the same offset.
    pub fn apply(&mut self, doc_text: &str, opts: &mut TransformOptions) {
        if !self.is_available() {
            return;
        }
        for (at, markup) in self.annotations(doc_text) {
            opts.annotations.insert(at, markup);
        }
    }
}

/// Run `logos unf <sentence> --json` and read the reply.
///
/// Every outcome other than a well-formed report becomes a [`Verdict`], never a
/// silent success: an unreadable answer means the document shows a red block
/// saying so, which is the outcome a reader can act on.
fn ask(binary: &Path, sentence: &str) -> Verdict {
    let output = Command::new(binary)
        .arg("unf")
        .arg(sentence)
        .arg("--json")
        .output();

    let out = match output {
        Ok(o) => o,
        Err(e) => return Verdict::Failed(format!("could not run {binary:?}: {e}")),
    };

    let stderr = String::from_utf8_lossy(&out.stderr);
    let stderr = stderr.trim();
    if !out.status.success() {
        // `logos unf` exits 1 for a rejected sentence and 2 for one that
        // compiled without a unique normal form. The distinction matters: the
        // second is a *content* failure with no fix, the first usually names an
        // out-of-lexicon word.
        if out.status.code() == Some(2) {
            return Verdict::NotConfluent {
                readback: "compiled without a unique normal form".into(),
            };
        }
        let reason = if stderr.is_empty() {
            format!("kernel exited {}", out.status)
        } else {
            stderr.to_string()
        };
        return Verdict::Failed(reason);
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    match serde_json::from_str::<serde_json::Value>(stdout.trim()) {
        Ok(v) => {
            let readback = v["result"].as_str().unwrap_or_default().to_string();
            let unf_hash = v["unf_hash"].as_str().unwrap_or_default().to_string();
            let verified = v["verified"].as_bool().unwrap_or(false);
            if unf_hash.len() != 64 {
                return Verdict::Failed(format!(
                    "kernel returned a {}-character unf_hash, expected 64",
                    unf_hash.len()
                ));
            }
            if verified {
                Verdict::Verified { readback, unf_hash }
            } else {
                Verdict::NotConfluent { readback }
            }
        }
        Err(e) => Verdict::Failed(format!("kernel reply is not JSON: {e}")),
    }
}

/// First `logos` on `$PATH`.
fn which() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(DEFAULT_BIN))
        .find(|c| is_executable(c))
}

/// Pin the kernel: `${KERNEL_BIN_VAR}` wins, then `$PATH`, then nothing.
///
/// An *empty* variable falls through to `$PATH` rather than pinning the
/// current directory, because `VAR= cmd` in a shell script is the natural way
/// to say "unset this" and honouring it would look for `./logos`.
///
/// Taken as an argument rather than read from the environment directly so the
/// precedence is unit-testable — `set_var` in a test would race every other
/// test in the process, and reading it here would make the rule untestable.
fn resolve_binary(env_value: Option<OsString>) -> Option<PathBuf> {
    env_value
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(which)
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// PathBuf of a binary name, for the CLI's diagnostics.
pub fn default_binary_name() -> OsString {
    OsString::from(DEFAULT_BIN)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in kernel: a shell script each test controls, so the transport is
    /// exercised without a real `logos`.
    ///
    /// Every call gets a **fresh directory**. Sharing one made these tests fail
    /// intermittently: the harness runs cases on concurrent threads, so two tests
    /// writing the same `logos` raced, and the loser either executed a
    /// half-written script or replaced the winner's. Writing to a staging name and
    /// renaming narrows that window but cannot close it — `rename` *replaces* the
    /// destination — so the fix is to stop sharing a path.
    fn fake_kernel(body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::atomic::{AtomicU32, Ordering};
        static SEQ: AtomicU32 = AtomicU32::new(0);

        let dir = std::env::temp_dir().join(format!(
            "emthin-fake-logos-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("create fake-kernel dir");
        let path = dir.join("logos");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write fake kernel");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fake kernel");

        // Exec the script once, right here, to make sure it can be executed at
        // all — and retry briefly if the filesystem is not ready.
        //
        // Without this these tests fail *intermittently* with ETXTBSY ("Text
        // file busy") on the exec the production code does a moment later:
        // writing a file and immediately exec'ing it can fail on a page cache
        // that has not published the inode yet. Sharing one path does not fix it
        // (it makes it worse — concurrent tests overwrite each other's script),
        // and the retry belongs here rather than in [`ask`], because
        // write-then-exec is a *fixture* hazard and production code never
        // compiles a program it just wrote.
        for attempt in 0..50 {
            match std::process::Command::new(&path).arg("--warm").output() {
                Ok(_) => break,
                Err(e) if e.raw_os_error() == Some(26) && attempt < 49 => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(e) => panic!("fake kernel {path:?} is not executable: {e}"),
            }
        }
        path
    }

    /// Remove a fake kernel's directory.
    ///
    /// Each script is a few dozen bytes and only this suite writes them, so
    /// leaving them would be a slow `$TMPDIR` leak rather than a correctness
    /// problem — but naming the cleanup in each test is also each test stating
    /// that it shares no path with any other.
    fn cleanup(path: &Path) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    fn hash() -> String {
        "600fbe115bf9d2788c7aaffbd64ff762762c48bb017d53338496b945ffa0d4e3".into()
    }

    fn reply(result: &str, verified: bool) -> String {
        format!(
            "echo '{{\"result\":\"{result}\",\"unf_hash\":\"{}\",\"verified\":{}}}'",
            hash(),
            verified
        )
    }

    #[test]
    fn a_verified_sentence_becomes_a_green_verdict() {
        let bin = fake_kernel(&reply("See(mary, bob)", true));
        let mut v = FormalVerifier::with_binary(Some(bin.clone()));
        let verdict = v.verify("Mary sees Bob").unwrap();
        assert_eq!(
            verdict,
            Verdict::Verified {
                readback: "See(mary, bob)".into(),
                unf_hash: hash(),
            }
        );
        assert!(verdict.is_verified());
        cleanup(&bin);
    }

    #[test]
    fn a_non_confluent_sentence_is_distinct_from_a_failure() {
        let bin = fake_kernel(&reply("See(mary, bob)", false));
        let mut v = FormalVerifier::with_binary(Some(bin.clone()));
        let verdict = v.verify("Mary sees Bob").unwrap();
        assert!(matches!(verdict, Verdict::NotConfluent { .. }));
        assert!(!verdict.is_verified());
        // "compiled but not unique" is not "did not compile".
        assert!(!matches!(verdict, Verdict::Failed(_)));
        cleanup(&bin);
    }

    /// Exit 2 is the kernel's own "no unique normal form", and it must not be
    /// reported as a crash.
    #[test]
    fn exit_two_is_reported_as_non_confluence() {
        let bin = fake_kernel("echo 'no unique normal form' >&2\nexit 2");
        let mut v = FormalVerifier::with_binary(Some(bin.clone()));
        let verdict = v.verify("x").unwrap();
        assert!(matches!(verdict, Verdict::NotConfluent { .. }));
        cleanup(&bin);
    }

    /// The kernel's reason must survive: an out-of-lexicon word is the whole
    /// actionable content of the failure.
    #[test]
    fn a_rejection_carries_the_kernel_reason() {
        let bin = fake_kernel("echo 'error: words not in the lexicon: Euler' >&2\nexit 1");
        let mut v = FormalVerifier::with_binary(Some(bin.clone()));
        match v.verify("Euler proves congruences").unwrap() {
            Verdict::Failed(reason) => assert!(reason.contains("Euler"), "{reason}"),
            other => panic!("expected Failed, got {other:?}"),
        }
        cleanup(&bin);
    }

    /// A binary that answers with something that is not a report must not be
    /// read as agreement.
    #[test]
    fn garbage_output_is_a_failure_not_a_pass() {
        let bin = fake_kernel("echo 'not json'");
        let mut v = FormalVerifier::with_binary(Some(bin.clone()));
        match v.verify("x").unwrap() {
            Verdict::Failed(reason) => assert!(reason.contains("JSON"), "{reason}"),
            other => panic!("expected Failed, got {other:?}"),
        }
        cleanup(&bin);
    }

    #[test]
    fn a_short_hash_is_a_failure() {
        let bin = fake_kernel("echo '{\"result\":\"R\",\"unf_hash\":\"abc\",\"verified\":true}'");
        let mut v = FormalVerifier::with_binary(Some(bin.clone()));
        match v.verify("x").unwrap() {
            Verdict::Failed(reason) => assert!(reason.contains("64"), "{reason}"),
            other => panic!("expected Failed, got {other:?}"),
        }
        cleanup(&bin);
    }

    /// A binary that cannot be executed is a failure for that sentence, not a
    /// panic — the kernel being broken is not the editor's problem.
    #[test]
    fn an_unrunnable_binary_is_a_failure() {
        let mut v = FormalVerifier::with_binary(Some(PathBuf::from("/nonexistent/logos")));
        match v.verify("x").unwrap() {
            Verdict::Failed(reason) => assert!(reason.contains("could not run"), "{reason}"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    // ── the no-op contract ────────────────────────────────────────────────

    /// No kernel means no annotations and no subprocesses — a checkout without
    /// `logos` must still open the editor.
    #[test]
    fn no_binary_is_a_silent_no_op() {
        let mut v = FormalVerifier::with_binary(None);
        assert!(!v.is_available());
        assert_eq!(v.calls(), 0);
        assert!(v.verify("anything").is_none());
        let doc = r#"#1 x #2 \formal(#1, #2, "x")"#;
        assert!(v.annotations(doc).is_empty());
        let mut opts = TransformOptions::default();
        v.apply(doc, &mut opts);
        assert!(opts.annotations.is_empty());
    }

    /// A document with no `\formal` statements must not spawn anything, however
    /// many times it is laid out.
    #[test]
    fn a_document_without_formals_never_calls_the_kernel() {
        let bin = fake_kernel(&reply("X", true));
        let mut v = FormalVerifier::with_binary(Some(bin.clone()));
        assert!(v.annotations("just prose, no statements here").is_empty());
        assert!(v.annotations("").is_empty());
        assert_eq!(v.calls(), 0);
        cleanup(&bin);
    }

    // ── the cache ─────────────────────────────────────────────────────────

    /// One subprocess per distinct sentence: the verdict is keyed by the CNL,
    /// so re-layout is free and two steps with one sentence share one answer.
    #[test]
    fn the_cache_is_keyed_on_the_sentence() {
        let bin = fake_kernel(&reply("R", true));
        let mut v = FormalVerifier::with_binary(Some(bin.clone()));
        let first = v.verify("same").unwrap();
        let second = v.verify("same").unwrap();
        assert_eq!(first, second);
        assert_eq!(v.calls(), 1, "a repeat must not respawn");
        cleanup(&bin);
    }

    // ── annotation assembly ───────────────────────────────────────────────

    #[test]
    fn steps_are_found_and_keyed_by_caption_start() {
        let doc = "#1 Mary sees Bob #2 \\formal(#1, #2, \"Mary sees Bob\") done";
        let steps = FormalVerifier::steps(doc);
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].spec.cnl, "Mary sees Bob");
        assert_eq!(steps[0].key, "g0");
        // The annotation key is the caption's *start*, which is where `transform`
        // inserts results and what `annotations` is keyed by.
        assert_eq!(
            &doc[steps[0].span.start..steps[0].span.end],
            " Mary sees Bob "
        );
    }

    #[test]
    fn apply_merges_verdicts_into_the_options() {
        let bin = fake_kernel(&reply("See(mary, bob)", true));
        let mut v = FormalVerifier::with_binary(Some(bin.clone()));
        let doc = "#1 Mary sees Bob #2 \\formal(#1, #2, \"Mary sees Bob\") done";
        let mut opts = TransformOptions::default();
        v.apply(doc, &mut opts);
        assert_eq!(opts.annotations.len(), 1);
        let (_at, markup) = opts.annotations.iter().next().unwrap();
        assert!(markup.contains("See(mary, bob)"), "{markup}");
        assert!(markup.contains(&hash()[..12]), "{markup}");
        cleanup(&bin);
    }

    /// The whole point of the seam: a `\formal` in a document typed into the
    /// compositor reaches the kernel during an ordinary relayout, with no
    /// explicit "verify" call anywhere in the frontend.
    #[test]
    fn relayout_asks_the_kernel_about_the_documents_formals() {
        let bin = fake_kernel(&reply("See(mary, bob)", true));
        let mut ui = crate::docui::DocUi::new();
        ui.formals_mut().set_binary(Some(bin.clone()));
        ui.model_mut().replace(
            0..0,
            "#1 Mary sees Bob #2 \\formal(#1, #2, \"Mary sees Bob\")",
        );
        ui.relayout();
        assert_eq!(ui.formals().calls(), 1, "the relayout must ask");
        assert!(ui.last_error().is_none(), "{:?}", ui.last_error());
        cleanup(&bin);
    }

    /// One sentence, two steps: the second one is answered from the cache.
    /// This is the transport half of the identity rule — two steps denoting one
    /// term get one answer, and therefore one UNF hash.
    #[test]
    fn one_sentence_in_two_steps_is_asked_once() {
        let bin = fake_kernel(&reply("See(mary, bob)", true));
        let mut ui = crate::docui::DocUi::new();
        ui.formals_mut().set_binary(Some(bin.clone()));
        ui.model_mut().replace(
            0..0,
            "#1 A #2 \\formal(#1, #2, \"See(mary, bob)\") #3 B #4 \\formal(#3, #4, \"See(mary, bob)\")",
        );
        ui.relayout();
        assert_eq!(ui.formals().calls(), 1);
        cleanup(&bin);
    }

    /// No kernel on `$PATH` must not break the editor: the declaration is laid
    /// out as written and no subprocess is attempted.
    #[test]
    fn a_missing_kernel_is_a_silent_no_op() {
        let mut ui = crate::docui::DocUi::new();
        ui.formals_mut().set_binary(None);
        assert!(!ui.formals().is_available());
        ui.model_mut()
            .replace(0..0, "#1 A #2 \\formal(#1, #2, \"See(mary, bob)\")");
        ui.relayout();
        assert_eq!(ui.formals().calls(), 0, "no binary, no subprocess");
        assert!(ui.last_error().is_none(), "{:?}", ui.last_error());
    }

    /// The override wins over `$PATH`, and an empty override falls through
    /// rather than pinning the current directory.
    #[test]
    fn the_env_override_beats_path_and_empty_falls_through() {
        assert_eq!(
            resolve_binary(Some(OsString::from("/opt/kernels/logos"))),
            Some(PathBuf::from("/opt/kernels/logos")),
            "an explicit override is used verbatim, even if it does not exist"
        );
        let empty = resolve_binary(Some(OsString::new()));
        // Whether that is `None` or a `$PATH` hit depends on the machine; what
        // must never happen is it resolving to `./logos`.
        assert!(
            empty.as_deref() != Some(Path::new("./logos")),
            "an empty override must not pin the current directory: {empty:?}"
        );
    }

    /// P7d: the proof DAG viewer is an ordinary figure, not a special pane.
    ///
    /// `logos formalize --vis dag.html` writes a self-contained page, so
    /// hosting it needs nothing from the compositor beyond what any app needs —
    /// a `\app` figure to show it in, and a remembered command to relaunch it:
    ///
    /// ```text
    /// #1 proof DAG #2 \app(#1, #2, 900, 600, "dag")
    /// #3 Mary sees Bob #4 \formal(#3, #4, "Mary sees Bob")
    /// ```
    ///
    /// Both kinds of statement coexist in one document and neither disturbs the
    /// other — which is the whole claim, so it is worth a test rather than a
    /// sentence in a changelog.
    #[test]
    fn the_dag_viewer_and_the_claim_are_figures_in_the_same_document() {
        let bin = fake_kernel(&reply("See(mary, bob)", true));
        let mut ui = crate::docui::DocUi::new();
        ui.formals_mut().set_binary(Some(bin.clone()));
        ui.set_viewport(smithay::utils::Size::from((1200, 1600)));
        ui.model_mut().replace(
            0..0,
            "#1 proof DAG #2 \\app(#1, #2, 900, 600, \"dag\", launch: \"foot file:///tmp/dag.html\")\n\
             #3 Mary sees Bob #4 \\formal(#3, #4, \"Mary sees Bob\")\n",
        );
        ui.relayout();
        assert!(ui.last_error().is_none(), "{:?}", ui.last_error());
        // The viewer is a figure with the usual `f<index>` key...
        assert!(
            ui.figures().get("f0").is_some(),
            "the \\app figure must survive alongside the \\formal step"
        );
        // ...the claim is not a figure at all, and was still verified.
        assert!(ui.figures().get("g0").is_none());
        assert_eq!(ui.formals().calls(), 1);
        // The relaunch command lives in the document, so a dormant DAG figure
        // comes back without any compositor state having to be kept in step.
        let (cmd, args) = ui.relaunch_target("f0").expect("the DAG viewer relaunches");
        assert_eq!(cmd, "foot");
        assert_eq!(args, vec!["file:///tmp/dag.html".to_string()]);
        cleanup(&bin);
    }

    /// The seam against the **real** kernel, not a stand-in.
    ///
    /// `logos unf` is the contract; everything above it in this file is a guess
    /// about that contract, and a guess that never meets the real thing is just
    /// a guess. Ignored by default because it needs a `logos` build:
    ///
    /// ```text
    /// cd ../unfer && cargo build -p logos --bin logos
    /// cd ../emthin && PATH="$PWD/../unfer/target/debug:$PATH" \
    ///     cargo test -p emthin --lib -- --ignored live_kernel
    /// ```
    ///
    /// `$PATH` rather than `${KERNEL_BIN_VAR}` because the override is
    /// [precedence-tested directly](resolve_binary) and these tests are about
    /// the subprocess contract.
    #[test]
    #[ignore = "needs a real logos build; see the doc comment for the invocation"]
    fn live_kernel_verifies_a_real_sentence() {
        let mut v = FormalVerifier::new();
        assert!(v.is_available(), "no logos on $PATH or ${KERNEL_BIN_VAR}");
        let verdict = v.verify("Mary sees Bob").expect("kernel present");
        assert!(
            matches!(verdict, Verdict::Verified { .. }),
            "a plain lexicon sentence must verify, got {verdict:?}"
        );
        let Verdict::Verified {
            readback, unf_hash, ..
        } = &verdict
        else {
            unreachable!()
        };
        assert_eq!(readback, "See(mary, bob)");
        assert_eq!(unf_hash.len(), 64, "the UNF hash is the pipeline identity");
        assert_eq!(v.calls(), 1);
    }

    /// An out-of-lexicon word is the failure mode the user actually hits, and it
    /// must arrive as the kernel's own words rather than a generic "failed".
    #[test]
    #[ignore = "needs a real logos build; see the doc comment for the invocation"]
    fn live_kernel_names_the_out_of_lexicon_word() {
        let mut v = FormalVerifier::new();
        assert!(v.is_available(), "no logos on $PATH or ${KERNEL_BIN_VAR}");
        let verdict = v.verify("Euler baptizes Bob").expect("kernel present");
        let Verdict::Failed(reason) = &verdict else {
            panic!("Euler is not in the stock lexicon, expected Failed, got {verdict:?}");
        };
        assert!(
            reason.contains("Euler"),
            "the kernel's reason must survive to the document: {reason}"
        );
    }

    /// The kernel is the authority on a CNL sentence, so `apply` overwrites
    /// rather than deferring. Pinned so the policy is a decision rather than an
    /// accident of `HashMap::insert` ordering — and the documented way for a
    /// frontend to win is to add its own annotation *after* calling `apply`.
    #[test]
    fn apply_overwrites_an_existing_annotation_at_the_same_offset() {
        let bin = fake_kernel(&reply("R", true));
        let mut v = FormalVerifier::with_binary(Some(bin.clone()));
        let doc = "#1 x #2 \\formal(#1, #2, \"x\") done";
        let scan = mathed_core::markers::scan(doc);
        let segments = mathed_core::markers::resolve_segments(&scan);
        let at = mathed_core::formalize::formals_in_segments(&segments)[0]
            .span
            .start;
        let mut opts = TransformOptions::default();
        opts.annotations.insert(at, "#text[mine]".into());
        v.apply(doc, &mut opts);
        let got = opts.annotations.get(&at).map(String::as_str).unwrap();
        assert!(
            got.contains("R"),
            "the kernel is the authority and replaces the old value: {got}"
        );
        cleanup(&bin);
    }
}
