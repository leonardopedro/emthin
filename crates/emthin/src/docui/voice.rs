//! The `\voice` dialogue block — the voice harness as an emthin plugin.
//!
//! This is the same shape as the two seams that came before it:
//!
//! - **`\slint`** (velyst's `mathed_core::slint` / `slint_blocks`) —
//!   a block marker in the document hands the heavy runtime to a helper
//!   process; the editor degrades to a blank block if the helper is gone.
//! - **`\formal`** ([`super::formals`]) — a subprocess per sentence, a
//!   seam resolved from `${EMTHIN_LOGOS_BIN}` or `$PATH`, and a no-op
//!   when the binary is missing.
//!
//! The voice plugin follows both: a `\voice` **block** in the document
//! is a dialogue session, the heavy runtime (LLM dialogue, mic, TTS)
//! lives in the `voice.sh` helper process (`${EMTHIN_VOICE_BIN}` or
//! `voice.sh` on `$PATH`), and if the helper is not installed every
//! turn is a logged rejection — the document still opens, lays out and
//! edits exactly as before.
//!
//! # Document syntax
//!
//! ```text
//! \voice typeyou: what is a Krylov basis?
//! gemma: It is a basis built by repeatedly applying a linear operator…
//! you: and a Lanczos iteration?
//! ```
//!
//! The first line is the header: the `\voice` marker (token-boundary
//! matched, so `\voicely` is prose) plus an optional mode word —
//! `type` (default), `speak`, `listen`, `full` — the same four modes as
//! `voice/voice.sh` itself, one per input/output combination:
//!
//! | mode | input | output |
//! |---|---|---|
//! | `type` (default) | typed `you:` line | text reply |
//! | `speak` | typed `you:` line | spoken reply |
//! | `listen` | microphone | text reply |
//! | `full` | microphone | spoken reply |
//!
//! Everything after the header is the **transcript**, and it lives in
//! the document — that is the whole design. The document is emthin's
//! session (`--session-file`), so the dialogue is persisted, editable,
//! undoable and visible with no second copy of the truth; the helper's
//! `--history` flag is fed *from the document transcript* each turn, so
//! editing a past turn edits the model's context too.
//!
//! The transform renders one hard newline as one `#linebreak()`
//! (foot-style, `mathed_core::transform`), so each turn line renders
//! on its own line without any block-splitting special case: a
//! transcript never contains a blank line, and a blank line ends the
//! block just as it does anywhere else in the document.
//!
//! # Interaction
//!
//! `Ctrl+Shift+V` ([`crate::docui::keymap::Action::VoiceTurn`]) or the
//! IPC `voice` method submits one turn:
//!
//! 1. if the transcript's last turn is an unanswered `you:` line, that
//!    text is asked about (the typed flow — nothing is re-typed);
//! 2. else, in `listen`/`full` blocks, the microphone is opened —
//!    `voice.sh dictate` records, transcribes and asks;
//! 3. else the press is rejected with a log line (type a `you:` line
//!    first — a text-only block never touches the microphone
//!    unasked).
//!
//! The reply is spliced into the block as a `gemma:` line by
//! [`poll`] on a later tick — the subprocess runs on a worker thread,
//! never on the event loop (a turn is seconds; a dictation is
//! `VOICE_SECS` of recording plus decode). Exactly one turn is in
//! flight at a time: the microphone is a single device, and replies
//! must land in order.
//!
//! A failed turn splices `voice: <reason>` into the transcript instead
//! of vanishing — the [`super::formals`] rule: a failure the reader
//! can see in the document is the outcome they can act on, and an
//! autosaved transcript that silently drops turns would lie.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

use mathed_core::blocks::split_blocks;

use crate::docui::model::DocModel;

/// Env var naming the harness, mirroring formals' `EMTHIN_LOGOS_BIN`.
pub const HARNESS_BIN_VAR: &str = "EMTHIN_VOICE_BIN";

/// Default binary name, resolved on `$PATH`.
const DEFAULT_BIN: &str = "voice.sh";

/// The block marker.
const MARKER: &str = "\\voice";

/// Longest line spliced into the document. Model replies are prose;
/// anything past this is a runaway generation, and a megabyte-long
/// line in an autosaved document is not something to persist.
const MAX_LINE: usize = 400;

// ---------------------------------------------------------------------------
// Block parsing
// ---------------------------------------------------------------------------

/// One of the four dialogue modes, matching `voice.sh`'s own words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceMode {
    /// Keyboard in, text out (the default).
    Type,
    /// Keyboard in, spoken reply.
    Speak,
    /// Microphone in, text out.
    Listen,
    /// Microphone in, spoken reply.
    Full,
}

impl VoiceMode {
    fn parse(word: &str) -> Option<Self> {
        match word {
            "type" => Some(Self::Type),
            "speak" => Some(Self::Speak),
            "listen" => Some(Self::Listen),
            "full" => Some(Self::Full),
            _ => None,
        }
    }

    /// Whether replies are spoken (`--say`): the `speak`/`full` output side.
    pub fn speaks_reply(self) -> bool {
        matches!(self, Self::Speak | Self::Full)
    }

    /// Whether the microphone may be opened for a turn with no typed
    /// question: the `listen`/`full` input side. A `type`/`speak`
    /// block never records without a typed `you:` line to answer.
    pub fn accepts_speech(self) -> bool {
        matches!(self, Self::Listen | Self::Full)
    }
}

/// A parsed `\voice` block: its mode and its transcript (the block
/// text after the header line).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceBlockSpec {
    pub mode: VoiceMode,
    pub body: String,
}

/// The text after the `\voice` marker, or `None` when `block` is not
/// a voice block. Token-boundary matched the same way slint is:
/// `\voicely` is prose, `\voice` and `\voice listen` are blocks.
fn marker_rest(block: &str) -> Option<&str> {
    let rest = block.strip_prefix(MARKER)?;
    match rest.chars().next() {
        None => Some(rest),
        Some(c) if c.is_whitespace() => Some(rest),
        _ => None,
    }
}

/// The full `\voice` block specification, or `None`.
///
/// The header is the first line: an optional mode word after the
/// marker, defaulting to [`VoiceMode::Type`]. Anything that is not a
/// mode word is left alone — a typo like `\voice listne` is a text
/// block rather than an error, because the document must never stop
/// laying out over a plugin's argument.
pub fn voice_block(block: &str) -> Option<VoiceBlockSpec> {
    let rest = marker_rest(block)?;
    let (header, body) = match rest.find('\n') {
        Some(n) => (&rest[..n], rest[n + 1..].to_string()),
        None => (rest, String::new()),
    };
    let mode = header
        .split_whitespace()
        .next()
        .and_then(VoiceMode::parse)
        .unwrap_or(VoiceMode::Type);
    Some(VoiceBlockSpec { mode, body })
}

/// Every `\voice` block in `text` with its byte range, in document order.
pub fn voice_blocks(text: &str) -> Vec<(Range<usize>, VoiceBlockSpec)> {
    split_blocks(text)
        .into_iter()
        .filter_map(|range| {
            let spec = voice_block(&text[range.clone()])?;
            Some((range, spec))
        })
        .collect()
}

/// The `\voice` block containing `pos` (both ends inclusive, so a
/// caret resting just after the last character still counts — it
/// does the moment after the user finishes typing a `you:` line).
pub fn voice_block_at(text: &str, pos: usize) -> Option<(Range<usize>, VoiceBlockSpec)> {
    voice_blocks(text)
        .into_iter()
        .find(|(range, _)| range.start <= pos && pos <= range.end)
}

// ---------------------------------------------------------------------------
// Transcript parsing
// ---------------------------------------------------------------------------

/// One parsed turn of the transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptTurn<'a> {
    /// `"user"` for a `you:` line, `"assistant"` for `gemma:`.
    pub role: &'a str,
    /// The turn's text; a following un-prefixed line continues it.
    pub content: String,
}

/// The content of `line` if it starts a turn with `prefix`
/// (`"you:"` / `"gemma:"`), trimmed. The colon is enough — `you:hi`
/// and `you: hi` both count, so a hand-typed transcript round-trips.
fn turn_start<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    line.strip_prefix(prefix).map(str::trim_start)
}

/// Lines that start a turn we must not mistake for content: the
/// transcript's own prefixes plus `=`, which `split_blocks` treats as
/// a heading and which would therefore end the block.
fn is_reserved_line(line: &str) -> bool {
    turn_start(line, "you:").is_some()
        || turn_start(line, "gemma:").is_some()
        || turn_start(line, "voice:").is_some()
        || line.starts_with('=')
}

/// Parse a block body into turns. Un-prefixed non-empty lines
/// continue the previous turn (multi-line replies survive);
/// `voice:` lines are harness notes and are not conversation.
pub fn transcript_turns(body: &str) -> Vec<TranscriptTurn<'_>> {
    let mut turns: Vec<TranscriptTurn> = Vec::new();
    for line in body.lines() {
        if let Some(content) = turn_start(line, "you:") {
            turns.push(TranscriptTurn {
                role: "user",
                content: content.to_string(),
            });
        } else if let Some(content) = turn_start(line, "gemma:") {
            turns.push(TranscriptTurn {
                role: "assistant",
                content: content.to_string(),
            });
        } else if turn_start(line, "voice:").is_some() {
            // A harness note (`voice: no reply from …`) is history
            // neither for the model nor for the reader's next turn.
            continue;
        } else if !line.trim().is_empty() {
            if let Some(prev) = turns.last_mut() {
                prev.content.push('\n');
                prev.content.push_str(line);
            }
        }
    }
    turns
}

/// The last turn of the transcript if it is a `you:` line with no
/// reply yet — the question the user already typed and wants answered.
///
/// Returns its index into [`transcript_turns`] together with the
/// text, so the caller can build the model history *before* it: the
/// harness appends the utterance itself, and a double-counted turn in
/// the context would make the model read its own question twice.
pub fn pending_turn(body: &str) -> Option<(usize, String)> {
    let turns = transcript_turns(body);
    let last = turns.last()?;
    (last.role == "user" && !last.content.trim().is_empty())
        .then(|| (turns.len() - 1, last.content.clone()))
}

/// The transcript as the helper's `--history` array.
///
/// `up_to` turns are included ([`None`] = all). The caller passes the
/// pending turn's index when the utterance being asked *is* that
/// turn, so the history holds everything before it and `voice.sh`'s
/// own `ask_llm` appends the question exactly once.
pub fn history_json(body: &str, up_to: Option<usize>) -> String {
    let turns = transcript_turns(body);
    let end = up_to.unwrap_or(turns.len()).min(turns.len());
    let arr: Vec<serde_json::Value> = turns[..end]
        .iter()
        .map(|t| serde_json::json!({ "role": t.role, "content": t.content }))
        .collect();
    serde_json::Value::Array(arr).to_string()
}

/// Clamp one line for splicing: no blank lines (a blank line would
/// end the block), no runaway length, and a leading turn prefix or
/// `=` neutralised with one space so a reply that quotes `you: …`
/// cannot masquerade as a turn when the transcript is parsed again.
fn clean_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut text = if trimmed.len() > MAX_LINE {
        let mut end = MAX_LINE;
        while !trimmed.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &trimmed[..end])
    } else {
        trimmed.to_string()
    };
    if is_reserved_line(&text) {
        text.insert(0, ' ');
    }
    Some(text)
}

/// The document lines a turn splices: `"<speaker>: "` in front of the
/// first line, continuations kept as their own lines (they render as
/// linebreaks and parse back into the same turn).
fn turn_lines(speaker: &str, text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let Some(clean) = clean_line(line) else {
            continue;
        };
        if out.is_empty() && i == 0 {
            out.push(format!("{speaker}: {clean}"));
        } else {
            out.push(clean);
        }
    }
    // `clean_line` may have dropped the first line (all-whitespace
    // reply): then no prefix was written and the speaker label is
    // missing from nothing, because there is nothing to splice.
    if out.is_empty() {
        return out;
    }
    if !out[0].starts_with(speaker) {
        out[0] = format!("{speaker}: {}", out[0].trim_start());
    }
    out
}

// ---------------------------------------------------------------------------
// The helper seam
// ---------------------------------------------------------------------------

/// Where to find the harness, and the one turn in flight.
///
/// Construction never probes beyond a `$PATH` scan — the first
/// [`VoiceBridge::submit`] decides whether the binary is usable, the
/// same deferral [`super::FormalVerifier`] uses.
pub struct VoiceBridge {
    binary: Option<PathBuf>,
    tx: Sender<VoiceEvent>,
    rx: Receiver<VoiceEvent>,
    /// One turn at a time: the microphone is a single device and
    /// replies must land in order. A second press while a turn runs
    /// is rejected, not queued.
    in_flight: bool,
    last_error: Option<String>,
}

impl Default for VoiceBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl VoiceBridge {
    pub fn new() -> Self {
        Self::with_binary(resolve_binary(std::env::var_os(HARNESS_BIN_VAR)))
    }

    /// A bridge pinned to one binary, for tests and pinned deployments.
    pub fn with_binary(binary: Option<PathBuf>) -> Self {
        let (tx, rx) = mpsc::channel();
        VoiceBridge {
            binary,
            tx,
            rx,
            in_flight: false,
            last_error: None,
        }
    }

    /// Whether a harness binary was found. Submitting without one is
    /// a rejection, never a crash — the slint rule.
    pub fn is_available(&self) -> bool {
        self.binary.is_some()
    }

    pub fn binary(&self) -> Option<&Path> {
        self.binary.as_deref()
    }

    /// Pin a different harness, or unpin it with `None`.
    pub fn set_binary(&mut self, binary: Option<PathBuf>) {
        if self.binary == binary {
            return;
        }
        self.binary = binary;
        // A turn spawned against the old binary may still land; it
        // would splice into the document under the new seam's nose.
        // Dropping in-flight events here is the conservative reading.
        while self.rx.try_recv().is_ok() {}
        self.in_flight = false;
    }

    /// A turn is running.
    pub fn in_flight(&self) -> bool {
        self.in_flight
    }

    /// The last turn's failure, if any.
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
}

/// `${EMTHIN_VOICE_BIN}` wins, then the first `voice.sh` on `$PATH`,
/// then nothing. An empty variable falls through to `$PATH` — the
/// same rule as [`super::formals`]' kernel resolution, and for the
/// same reason: `VAR= cmd` means "unset this", not "pin `./voice.sh`".
fn resolve_binary(env_value: Option<std::ffi::OsString>) -> Option<PathBuf> {
    env_value
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(which)
}

fn which() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(DEFAULT_BIN))
        .find(|c| is_executable(c))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Submitting a turn
// ---------------------------------------------------------------------------

/// Which block a turn targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// The block containing the caret — what `Ctrl+Shift+V` means.
    Caret,
    /// The n-th `\voice` block in document order (IPC `block`).
    Block(usize),
    /// IPC with no `block`: the caret's block if it has one, else the
    /// first — a script driving the document has no caret of its own.
    CaretOrFirst,
}

/// A queued turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceSubmit {
    /// Index of the target block among the document's `\voice` blocks.
    pub block_index: usize,
    /// The turn came from the microphone or an explicit IPC text, so
    /// the `you:` line is **not** in the document yet and must be
    /// spliced together with the reply.
    pub append_utterance: bool,
    /// Replies are spoken for this block's mode (`--say`).
    pub say: bool,
}

/// Why one turn's outcome came back as it did.
#[derive(Debug, Clone)]
pub struct VoiceTurn {
    /// The question — typed from the document, dictated, or passed
    /// over IPC. Always reported even when it was never spliced (the
    /// typed flow leaves the user's own `you:` line alone).
    pub utterance: String,
    pub reply: String,
}

/// A finished turn, delivered by the worker thread.
#[derive(Debug, Clone)]
pub struct VoiceEvent {
    /// Byte offset of the target block when the turn was submitted —
    /// the re-find key when the reply lands.
    pub block_start: usize,
    pub block_index: usize,
    /// The turn came from the microphone or an explicit IPC text, so
    /// its `you:` line is not in the document yet and must be spliced
    /// alongside the reply.
    pub append_utterance: bool,
    pub outcome: Result<VoiceTurn, String>,
}

/// Serialises history-file names; unique per process without a lock.
static HISTORY_SEQ: AtomicU64 = AtomicU64::new(0);

/// Queue one turn for `target` against the harness.
///
/// Pure decision + spawn: the subprocess runs on a worker thread and
/// its outcome arrives through [`poll`] on a later tick, so the event
/// loop never blocks on a model round trip or a recording.
pub fn submit(
    model: &DocModel,
    bridge: &mut VoiceBridge,
    target: Target,
    explicit_text: Option<&str>,
) -> Result<VoiceSubmit, String> {
    if bridge.in_flight {
        return Err("a voice turn is already running".into());
    }
    let binary = bridge.binary.clone().ok_or_else(|| {
        format!("no voice harness (set {HARNESS_BIN_VAR} or put {DEFAULT_BIN} on PATH)")
    })?;

    let text = model.text();
    let blocks = voice_blocks(text);
    if blocks.is_empty() {
        return Err("the document has no \\voice block".into());
    }
    let caret = model.caret();
    let index_of = |r: &Range<usize>| blocks.iter().position(|(br, _)| br == r).unwrap_or(0);
    let (block_index, range, spec) = match target {
        Target::Caret => {
            let (r, s) = voice_block_at(text, caret)
                .ok_or_else(|| "the caret is not in a \\voice block".to_string())?;
            (index_of(&r), r, s)
        }
        Target::Block(i) => {
            let (r, s) = blocks
                .get(i)
                .cloned()
                .ok_or_else(|| format!("no \\voice block #{i}"))?;
            (i, r, s)
        }
        Target::CaretOrFirst => match voice_block_at(text, caret) {
            Some((r, s)) => (index_of(&r), r, s),
            None => {
                let (r, s) = blocks[0].clone();
                (0, r, s)
            }
        },
    };

    // Decide what is being asked, and from which history.
    let body = &spec.body;
    let pending = pending_turn(body);
    // The pending turn's index, kept apart because the match below
    // takes its text by value.
    let pending_index = pending.as_ref().map(|(idx, _)| *idx);
    enum Ask {
        /// The typed `you:` line; history stops before it.
        Pending(String),
        /// An explicit IPC utterance; history is the whole body.
        Text(String),
        /// The microphone; history is the whole body.
        Dictate,
    }
    let ask =
        match explicit_text {
            Some(t) if !t.trim().is_empty() => Ask::Text(t.to_string()),
            Some(_) => return Err("empty utterance".into()),
            None => match pending {
                Some((_, content)) => Ask::Pending(content),
                None if spec.mode.accepts_speech() => Ask::Dictate,
                None => return Err(
                    "no pending `you:` line to answer (type one, or use a `listen`/`full` block)"
                        .into(),
                ),
            },
        };

    // The history the harness sees: the document's transcript up to
    // the turn being asked, so the document stays the single source
    // of truth and an edit above the question edits its context.
    let up_to = match &ask {
        Ask::Pending(_) => pending_index,
        Ask::Text(_) | Ask::Dictate => None,
    };
    let history = history_json(body, up_to);

    let seq = HISTORY_SEQ.fetch_add(1, Ordering::Relaxed);
    let history_path = std::env::temp_dir().join(format!(
        "emthin-voice-hist-{}-{seq}.json",
        std::process::id()
    ));
    std::fs::write(&history_path, history)
        .map_err(|e| format!("could not write {}: {e}", history_path.display()))?;

    let mut args: Vec<String> = Vec::new();
    args.push(
        match &ask {
            Ask::Dictate => "dictate",
            Ask::Pending(_) | Ask::Text(_) => "turn",
        }
        .to_string(),
    );
    if spec.mode.speaks_reply() {
        args.push("--say".into());
    }
    args.push("--history".into());
    args.push(history_path.to_string_lossy().into_owned());
    let utterance = match &ask {
        Ask::Dictate => String::new(),
        Ask::Pending(u) | Ask::Text(u) => u.clone(),
    };
    if !utterance.is_empty() {
        args.push(utterance.clone());
    }
    let append_utterance = matches!(ask, Ask::Dictate | Ask::Text(_));

    let block_start = range.start;
    let say = spec.mode.speaks_reply();
    spawn_turn(
        bridge,
        TurnJob {
            binary,
            args,
            history_path,
            block_start,
            block_index,
            append_utterance,
            utterance,
        },
    );
    Ok(VoiceSubmit {
        block_index,
        append_utterance,
        say,
    })
}

/// Everything one worker turn needs, bundled so [`spawn_turn`] takes
/// a single argument and the event's fields live in one place.
struct TurnJob {
    binary: PathBuf,
    args: Vec<String>,
    /// Written *before* the spawn (the child may read it immediately)
    /// and removed by the worker once the child is done, so a turn
    /// leaves no state behind but the transcript it splices.
    history_path: PathBuf,
    block_start: usize,
    block_index: usize,
    append_utterance: bool,
    /// The submitted question — empty for a dictation, which learns
    /// its question from the harness's echo.
    utterance: String,
}

/// Run the harness on a worker thread and post the outcome.
fn spawn_turn(bridge: &mut VoiceBridge, job: TurnJob) {
    bridge.in_flight = true;
    let tx = bridge.tx.clone();
    std::thread::spawn(move || {
        let outcome = run_harness(&job.binary, &job.args).map(|(reported, reply)| {
            // The caller knows what it asked; only a dictation (empty
            // submitted text) learns its question from the harness's
            // echo. A harness that echoes something else cannot
            // rewrite a turn the document already contains.
            let utterance = if job.utterance.trim().is_empty() {
                reported
            } else {
                job.utterance.clone()
            };
            VoiceTurn { utterance, reply }
        });
        let _ = std::fs::remove_file(&job.history_path);
        let _ = tx.send(VoiceEvent {
            block_start: job.block_start,
            block_index: job.block_index,
            append_utterance: job.append_utterance,
            outcome,
        });
    });
}

/// Run `voice.sh turn|dictate …` and read `(utterance, reply)`.
///
/// Every outcome other than well-formed JSON on stdout becomes an
/// `Err` — the same rule as [`super::formals`]' `ask`: an unreadable
/// answer must be visible, never a silent success.
fn run_harness(binary: &Path, args: &[String]) -> Result<(String, String), String> {
    let output = Command::new(binary)
        .args(args)
        .output()
        .map_err(|e| format!("could not run {binary:?}: {e}"))?;
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() {
        let reason = if stderr.is_empty() {
            format!("harness exited {}", output.status)
        } else {
            // `voice.sh` writes `FAIL <reason>` last; curl/record noise
            // above it is context, so keep both but bound the whole.
            let mut s = stderr;
            if s.len() > 400 {
                let mut end = 400;
                while !s.is_char_boundary(end) {
                    end -= 1;
                }
                s.truncate(end);
                s.push('…');
            }
            s
        };
        return Err(reason.replace('\n', " · "));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim())
        .map_err(|e| format!("harness reply is not JSON: {e}"))?;
    let reply = value["reply"].as_str().unwrap_or_default().to_string();
    if reply.trim().is_empty() {
        return Err("harness returned an empty reply".into());
    }
    let utterance = value["utterance"].as_str().unwrap_or_default().to_string();
    Ok((utterance, reply))
}

// ---------------------------------------------------------------------------
// Polling: finishing a turn into the document
// ---------------------------------------------------------------------------

/// Drain finished turns into the document and return them.
///
/// Called once per tick *before* the relayout so a reply that lands
/// this tick is typeset this tick. Each splice is one undo step
/// ([`DocModel::replace_keeping_caret`]), so `Ctrl+Z` takes a reply
/// out of the transcript without touching the caret.
pub fn poll(model: &mut DocModel, bridge: &mut VoiceBridge) -> Vec<VoiceEvent> {
    let mut events = Vec::new();
    while let Ok(event) = bridge.rx.try_recv() {
        bridge.in_flight = false;
        match &event.outcome {
            Ok(turn) => {
                let mut lines = Vec::new();
                // The typed flow's `you:` line is already in the
                // document; dictated and IPC turns bring their own.
                if event.append_utterance {
                    lines.extend(turn_lines("you", &turn.utterance));
                }
                lines.extend(turn_lines("gemma", &turn.reply));
                splice(model, event.block_start, &lines);
                tracing::info!(
                    "voice turn landed in block #{}: {} chars",
                    event.block_index,
                    turn.reply.chars().count()
                );
            }
            Err(message) => {
                bridge.last_error = Some(message.clone());
                let lines = turn_lines("voice", message);
                splice(model, event.block_start, &lines);
                tracing::warn!("voice turn failed: {message}");
            }
        }
        events.push(event);
    }
    events
}

/// Insert `lines` at the end of the block nearest `preferred`.
///
/// Nearest rather than exact: the document may have been edited while
/// the turn ran (a paragraph typed above it, an IPC edit), so the
/// recorded offset shifts with it. A bound keeps that honest — with
/// no bound, deleting the block would let its late reply land in
/// whatever block survives nearest the old offset, and a wholesale
/// document replacement would pour one conversation's answer into
/// another's transcript. If the block is gone entirely the reply is
/// dropped with a warning.
///
/// Within the bound, proximity is the best identity available: `\voice`
/// blocks carry no marker pair the way `\app` figures do, so an
/// edited-around block and a replaced one cannot be told apart by
/// offset alone. Dropping on distance is the conservative reading.
fn splice(model: &mut DocModel, preferred: usize, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    let text = model.text();
    const RE_FIND_BOUND: usize = 4096;
    let Some((range, _)) = voice_blocks(text)
        .into_iter()
        .min_by_key(|(r, _)| r.start.abs_diff(preferred))
        .filter(|(r, _)| r.start.abs_diff(preferred) <= RE_FIND_BOUND)
    else {
        tracing::warn!("voice: target block near {preferred} is gone; reply dropped");
        return;
    };
    let mut insert = String::from("\n");
    insert.push_str(&lines.join("\n"));
    model.replace_keeping_caret(range.end..range.end, &insert);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in harness: a shell script each test controls, so the
    /// transport, the argument contract and the splice are exercised
    /// without a real `voice.sh` (or a model, a microphone, a sound
    /// server).
    ///
    /// Every call gets a **fresh directory**, for the same reason
    /// `formals`' fake kernel does: the harness runs tests on
    /// concurrent threads, and a shared path races.
    struct FakeHarness {
        dir: PathBuf,
        script: PathBuf,
    }

    impl FakeHarness {
        fn new(name: &str, body: &str) -> Self {
            use std::os::unix::fs::PermissionsExt;
            static SEQ: AtomicU64 = AtomicU64::new(0);
            let seq = SEQ.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "emthin-voice-fake-{name}-{}-{seq}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).expect("create temp dir");
            let script = dir.join("voice.sh");
            std::fs::write(&script, format!("#!/bin/sh\n{body}\n")).expect("write script");
            let mut perms = std::fs::metadata(&script).expect("stat").permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script, perms).expect("chmod");
            FakeHarness { dir, script }
        }

        /// A harness that answers `turn`/`dictate` with fixed text and
        /// records its argv (and the history file) beside itself. The
        /// argument layout the real harness promises is assumed:
        /// `turn|dictate [--say] --history FILE [TEXT]` — the history
        /// path is found by scanning for `--history` because `--say`
        /// shifts every position behind it.
        ///
        /// The echo (`utterance`) repeats the same fixed text: a
        /// dictation learns its `you:` line from the echo, so the fake
        /// "hears" what it also answers. A placeholder echo would splice
        /// a meaningless heard line into every dictate-mode assertion.
        fn answering(reply: &str) -> Self {
            let mut body = String::from("echo \"$@\" > \"$0.args\"\n");
            body.push_str("prev=''; for a in \"$@\"; do \n");
            body.push_str("  if [ \"$prev\" = --history ] && [ -f \"$a\" ]; then cp \"$a\" \"$0.history\"; fi\n");
            body.push_str("  prev=\"$a\"\ndone\n");
            body.push_str(&format!(
                "printf '{{\"utterance\": \"{}\", \"reply\": \"{}\"}}'\n",
                reply.replace('"', "\\\""),
                reply.replace('"', "\\\"")
            ));
            FakeHarness::new("ok", &body)
        }

        fn failing(message: &str) -> Self {
            let body = format!("echo \"$@\" > \"$0.args\"\nprintf 'FAIL {message}' >&2\nexit 1");
            Self::new("fail", &body)
        }

        fn args(&self) -> String {
            std::fs::read_to_string(self.script.with_extension("sh.args"))
                .or_else(|_| std::fs::read_to_string(format!("{}.args", self.script.display())))
                .unwrap_or_default()
        }

        fn history(&self) -> String {
            std::fs::read_to_string(format!("{}.history", self.script.display()))
                .unwrap_or_default()
        }

        fn bridge(&self) -> VoiceBridge {
            VoiceBridge::with_binary(Some(self.script.clone()))
        }
    }

    impl Drop for FakeHarness {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// Poll until the worker posts its outcome (or the deadline).
    fn settle(model: &mut DocModel, bridge: &mut VoiceBridge) -> Vec<VoiceEvent> {
        let mut events = Vec::new();
        for _ in 0..500 {
            events.extend(poll(model, bridge));
            if !events.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!events.is_empty(), "the worker never posted an outcome");
        events
    }

    // -- block parsing -----------------------------------------------------

    #[test]
    fn a_voice_block_carries_its_mode_and_body() {
        let spec = voice_block("\\voice listen\nyou: hello").expect("a voice block");
        assert_eq!(spec.mode, VoiceMode::Listen);
        assert_eq!(spec.body, "you: hello");
    }

    #[test]
    fn the_marker_needs_a_token_boundary() {
        assert!(voice_block("\\voice").is_some(), "bare marker is a block");
        assert!(
            voice_block("\\voice\nyou: hi").is_some(),
            "newline boundary"
        );
        assert!(
            voice_block("\\voicely prose").is_none(),
            "\\voicely is a word"
        );
        assert!(
            voice_block("prose \\voice hi").is_none(),
            "not at block start"
        );
    }

    #[test]
    fn an_unknown_mode_word_falls_back_to_type() {
        let spec = voice_block("\\voice listne\nyou: hi").expect("a block");
        assert_eq!(spec.mode, VoiceMode::Type, "a typo must not break the doc");
        assert_eq!(spec.body, "you: hi");
    }

    #[test]
    fn a_headerless_block_is_text_mode_with_the_body_below() {
        let spec = voice_block("\\voice\nyou: hi").expect("a block");
        assert_eq!(spec.mode, VoiceMode::Type);
        assert_eq!(spec.body, "you: hi");
    }

    #[test]
    fn blocks_are_found_in_document_order() {
        let text = "\\voice type\nyou: one\n\nprose in between\n\n\\voice full\nyou: two";
        let blocks = voice_blocks(text);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].1.mode, VoiceMode::Type);
        assert_eq!(blocks[1].1.mode, VoiceMode::Full);
        assert_eq!(blocks[0].1.body, "you: one");
        assert_eq!(blocks[1].1.body, "you: two");
    }

    // -- transcript parsing ------------------------------------------------

    #[test]
    fn the_pending_turn_is_the_unanswered_you_line() {
        let body = "you: first question\ngemma: the answer\nyou: second question";
        let (idx, text) = pending_turn(body).expect("pending");
        assert_eq!(idx, 2);
        assert_eq!(text, "second question");
    }

    #[test]
    fn a_replied_transcript_has_no_pending_turn() {
        assert!(
            pending_turn("you: hi\ngemma: hello").is_none(),
            "an answered question must not be re-asked"
        );
        assert!(pending_turn("").is_none());
        assert!(pending_turn("gemma: unprompted").is_none());
    }

    #[test]
    fn unattributed_lines_continue_the_previous_turn() {
        let turns = transcript_turns("you: line one\nstill mine\ngemma: line one\nstill mine");
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].content, "line one\nstill mine");
        assert_eq!(turns[1].content, "line one\nstill mine");
    }

    #[test]
    fn harness_notes_are_not_conversation() {
        let turns = transcript_turns("you: hi\ngemma: hey\nvoice: no reply from the model");
        assert_eq!(turns.len(), 2, "voice: lines must not reach the model");
    }

    #[test]
    fn history_stops_before_the_turn_being_asked() {
        let body = "you: first\ngemma: answer\nyou: second";
        let all: serde_json::Value =
            serde_json::from_str(&history_json(body, None)).expect("json array");
        assert_eq!(
            all.as_array().expect("array").len(),
            3,
            "the whole transcript is three turns"
        );
        // The pending turn is excluded: `voice.sh` appends it itself.
        let before: serde_json::Value =
            serde_json::from_str(&history_json(body, Some(2))).expect("json array");
        let arr = before.as_array().expect("array");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[1]["role"], "assistant");
        // …and the roles are the OpenAI spellings the harness wants.
        assert_eq!(arr[0]["role"], "user");
        assert_eq!(arr[0]["content"], "first");
    }

    // -- splicing text -----------------------------------------------------

    #[test]
    fn a_reply_cannot_end_the_block_or_forge_a_turn() {
        let lines = turn_lines("gemma", "a\n\nb\nyou: forged\ngemma: forged\n=forged");
        // Blank line dropped — it would end the block.
        assert!(!lines.iter().any(|l| l.trim().is_empty()), "{lines:?}");
        // Reserved prefixes are space-prefixed, so parsing them back
        // yields continuations, not turns.
        assert_eq!(lines[0], "gemma: a");
        assert!(
            lines.iter().any(|l| l.starts_with(" you: forged")),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.starts_with(" gemma: forged")),
            "{lines:?}"
        );
        assert!(lines.iter().any(|l| l.starts_with(" =forged")), "{lines:?}");
    }

    #[test]
    fn a_runaway_line_is_clamped_on_a_char_boundary() {
        let long = "é".repeat(500);
        let lines = turn_lines("gemma", &long);
        assert_eq!(lines.len(), 1);
        assert!(
            lines[0].chars().count() < MAX_LINE + 10,
            "{} chars",
            lines[0].chars().count()
        );
        assert!(lines[0].starts_with("gemma: "));
    }

    #[test]
    fn an_empty_reply_splices_nothing() {
        assert!(turn_lines("gemma", "  \n ").is_empty());
    }

    // -- the seam ----------------------------------------------------------

    #[test]
    fn no_binary_means_a_rejection_not_a_crash() {
        let mut bridge = VoiceBridge::with_binary(None);
        let mut model = DocModel::new("\\voice\nyou: hi");
        model.take_dirty(); // the constructor marks a fresh model dirty
        let err = submit(&model, &mut bridge, Target::Caret, None).unwrap_err();
        assert!(err.contains("no voice harness"), "{err}");
        assert!(!model.is_dirty(), "a rejection must not dirty the document");
    }

    #[test]
    fn an_empty_env_var_falls_through_to_path() {
        assert_eq!(
            resolve_binary(Some(std::ffi::OsString::new())),
            which(),
            "VAR= cmd means unset, not ./voice.sh"
        );
        assert_eq!(
            resolve_binary(Some(std::ffi::OsString::from("/pinned/voice.sh"))),
            Some(PathBuf::from("/pinned/voice.sh"))
        );
    }

    #[test]
    fn a_text_block_without_a_pending_line_is_rejected() {
        let fake = FakeHarness::answering("irrelevant");
        let mut bridge = fake.bridge();
        let mut model = DocModel::new("\\voice type\nyou: hi\ngemma: already answered");
        model.set_caret(model.text().find("gemma").expect("caret in block"));
        let err = submit(&model, &mut bridge, Target::Caret, None).unwrap_err();
        assert!(err.contains("no pending"), "{err}");
        assert!(!bridge.in_flight(), "nothing must have been spawned");
    }

    // -- the round trip ----------------------------------------------------

    #[test]
    fn a_typed_question_gets_its_gemma_line_spliced() {
        let fake = FakeHarness::answering("fine, thanks");
        let mut bridge = fake.bridge();
        let mut model = DocModel::new("\\voice type\nyou: how are you?");
        model.set_caret(model.text().find("you:").expect("caret at the question"));

        let submit_info = submit(&model, &mut bridge, Target::Caret, None).expect("queued");
        assert!(
            !submit_info.append_utterance,
            "the you: line is already there"
        );
        assert!(!submit_info.say, "type mode does not speak");
        assert!(bridge.in_flight());

        let events = settle(&mut model, &mut bridge);
        assert!(events[0].outcome.is_ok());
        assert_eq!(
            model.text(),
            "\\voice type\nyou: how are you?\ngemma: fine, thanks",
            "the reply is spliced into the block, and nothing else changed"
        );
        assert!(!bridge.in_flight(), "the bridge is reusable after a turn");
    }

    #[test]
    fn the_history_the_harness_sees_is_the_document_up_to_the_ask() {
        let fake = FakeHarness::answering("two");
        let mut bridge = fake.bridge();
        let doc = "\\voice type\nyou: first question\ngemma: first answer\nyou: second question";
        let mut model = DocModel::new(doc);
        model.set_caret(doc.len());

        submit(&model, &mut bridge, Target::Caret, None).expect("queued");
        settle(&mut model, &mut bridge);

        let history = fake.history();
        let parsed: serde_json::Value =
            serde_json::from_str(&history).expect("history is one JSON array");
        let arr = parsed.as_array().expect("array");
        assert_eq!(
            arr.len(),
            2,
            "history stops before the pending turn: {history}"
        );
        assert_eq!(arr[0]["content"], "first question");
        assert_eq!(arr[1]["role"], "assistant");
        // The argument contract: `turn [--say] --history FILE TEXT`.
        let args = fake.args();
        assert!(args.starts_with("turn "), "{args}");
        assert!(args.contains("--history"), "{args}");
        assert!(args.contains("second question"), "{args}");
        assert!(!args.contains("--say"), "type mode never speaks: {args}");
    }

    #[test]
    fn a_listen_block_with_nothing_typed_opens_the_microphone() {
        let fake = FakeHarness::answering("heard you");
        let mut bridge = fake.bridge();
        let mut model = DocModel::new("\\voice listen\nyou: old\ngemma: old answer");
        model.set_caret(0);

        let info = submit(&model, &mut bridge, Target::Caret, None).expect("queued");
        assert!(info.append_utterance, "the heard line must be spliced");
        assert!(!info.say, "listen replies in text");
        settle(&mut model, &mut bridge);

        let args = fake.args();
        assert!(args.starts_with("dictate"), "{args}");
        assert_eq!(
            model.text(),
            "\\voice listen\nyou: old\ngemma: old answer\nyou: heard you\ngemma: heard you",
            "a dictated turn splices both lines"
        );
    }

    #[test]
    fn a_full_block_dictates_and_speaks() {
        let fake = FakeHarness::answering("spoken");
        let mut bridge = fake.bridge();
        let mut model = DocModel::new("\\voice full");
        model.set_caret(0);

        let info = submit(&model, &mut bridge, Target::Caret, None).expect("queued");
        assert!(info.say, "full mode speaks replies");
        settle(&mut model, &mut bridge);
        assert!(fake.args().contains("--say"), "{}", fake.args());
    }

    #[test]
    fn an_ipc_text_turn_brings_its_own_you_line() {
        let fake = FakeHarness::answering("direct answer");
        let mut bridge = fake.bridge();
        let mut model = DocModel::new("\\voice type\ngemma: earlier");
        model.set_caret(0);

        let info = submit(
            &model,
            &mut bridge,
            Target::Block(0),
            Some("asked over IPC"),
        )
        .expect("queued");
        assert!(info.append_utterance);
        settle(&mut model, &mut bridge);
        assert_eq!(
            model.text(),
            "\\voice type\ngemma: earlier\nyou: asked over IPC\ngemma: direct answer"
        );
    }

    #[test]
    fn a_failed_turn_is_visible_in_the_transcript() {
        let fake = FakeHarness::failing("model is down");
        let mut bridge = fake.bridge();
        let mut model = DocModel::new("\\voice type\nyou: hi");
        model.set_caret(model.text().len());

        submit(&model, &mut bridge, Target::Caret, None).expect("queued");
        let events = settle(&mut model, &mut bridge);
        assert!(events[0].outcome.is_err());
        assert_eq!(
            model.text(),
            "\\voice type\nyou: hi\nvoice: FAIL model is down",
            "the reader must see why there is no answer"
        );
        assert!(bridge
            .last_error()
            .expect("recorded")
            .contains("model is down"));
    }

    #[test]
    fn only_one_turn_is_in_flight() {
        let fake = FakeHarness::answering("slow");
        let mut bridge = fake.bridge();
        let mut model = DocModel::new("\\voice type\nyou: hi");
        model.set_caret(model.text().len());
        submit(&model, &mut bridge, Target::Caret, None).expect("first queued");
        let err = submit(&model, &mut bridge, Target::Caret, None).unwrap_err();
        assert!(err.contains("already running"), "{err}");
        settle(&mut model, &mut bridge);
    }

    #[test]
    fn a_deleted_block_drops_its_late_reply_instead_of_landing_elsewhere() {
        let fake = FakeHarness::answering("orphan");
        let mut bridge = fake.bridge();
        let mut model = DocModel::new("\\voice type\nyou: hi");
        model.set_caret(model.text().len());
        submit(&model, &mut bridge, Target::Caret, None).expect("queued");

        // The document is replaced wholesale while the turn runs (an
        // IPC `open_doc`, say): the block the reply belongs to is
        // gone, and no `\voice` block survives anywhere near its
        // recorded offset.
        model.replace(0..model.text().len(), "just prose now");
        let events = settle(&mut model, &mut bridge);
        assert!(events[0].outcome.is_ok(), "the turn itself succeeded");
        assert_eq!(
            model.text(),
            "just prose now",
            "a reply must not land in an unrelated document on a stale offset"
        );
    }

    #[test]
    fn targeting_falls_back_to_the_first_block_over_ipc() {
        let fake = FakeHarness::answering("fallback");
        let mut bridge = fake.bridge();
        // The caret sits in the prose block, not in a voice one.
        let doc = "some prose\n\n\\voice type\nyou: one";
        let mut model = DocModel::new(doc);
        model.set_caret(0);

        let info = submit(&model, &mut bridge, Target::CaretOrFirst, None).expect("queued");
        assert_eq!(info.block_index, 0);
        settle(&mut model, &mut bridge);
        assert!(
            model.text().contains("you: one\ngemma: fallback"),
            "{}",
            model.text()
        );
    }
}
