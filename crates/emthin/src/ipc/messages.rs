//! The emthin control protocol: JSON-RPC 2.0 notifications in and out.
//!
//! This replaced the Emacs-driven message set (which existed only to
//! let an Elisp layout engine drive geometry). The compositor is now
//! self-contained: it lays out its own document, and this protocol is
//! an **observation and command** surface — enough to drive it from
//! outside (a bar, a script, a future external-shell frontend) without
//! re-deriving its state.
//!
//! See `docs/ipc.md` for the wire description.
//!
//! ## Wire format
//!
//! `Content-Length: N\r\n\r\n` + JSON body, one JSON object per frame,
//! all notifications (no `id`, no responses). Conversions are written
//! by hand rather than derived from serde: `OutgoingMessage::method_name`
//! is the one place that decides the wire spelling of each event, and a
//! derive would let a Rust rename silently break every client.

use crate::docui::file::SaveFormat;

/// A rectangle in **output-local logical pixels**, not fractions.
///
/// The old protocol spoke `f64` fractions of the Emacs frame because
/// an external layout engine didn't know the pixel size. The document
/// now decides geometry in absolute points (Typst's unit), so the
/// control plane reports absolute pixels too — no lossy round-trip
/// through a ratio.
///
/// [`crate::ipc`]: crate::ipc
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct IpcRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Control client → emthin.
#[derive(Debug)]
pub enum IncomingMessage {
    /// Launch a program into a (new or existing) figure.
    Spawn { cmd: String, args: Vec<String> },
    /// Close the app bound to a figure. The `\app` statement stays, so
    /// the figure goes dormant rather than disappearing.
    Close { figure: String },
    /// Move keyboard focus to a figure (`figure: null` = the document).
    Focus { figure: Option<String> },
    /// Rewrite a figure's `\app` width/height arguments.
    SetFigureSize { figure: String, w: i32, h: i32 },
    /// Duplicate a figure's statement into a new mirror figure.
    CloneFigure { figure: String },
    /// Change the visible page (0-based).
    GotoPage { page: usize },
    /// Open a document from disk (replacing the current one).
    OpenDoc { path: String },
    /// Write the current document to a path. `format` defaults to the path
    /// extension (`.loro`/`.mathed` = snapshot, otherwise source). `force`
    /// replaces a file that changed since it was opened or last saved.
    SaveDoc {
        path: String,
        format: Option<SaveFormat>,
        force: bool,
    },
    /// Write the document's ASCII-only Typst interchange projection.
    ExportAscii { path: String },
    /// Replace the document with a file containing the ASCII-only projection.
    ImportAscii { path: String },
    /// Set the document's reading size, in points. Values outside the
    /// supported range are clamped; the resulting size is reported in
    /// `text_size_changed`.
    SetTextSize { pt: f32 },
    /// Feed one synthetic touch contact into the live gesture handlers. This
    /// is the test seam for two-finger panning: the live harness has no
    /// touchscreen, and XTEST cannot synthesize multitouch. Coordinates are
    /// output-local logical pixels, the same space pointer events use.
    InjectTouch {
        phase: TouchPhase,
        slot: i32,
        x: f64,
        y: f64,
    },
    /// Dump the full document + figure state.
    ListState,
    /// Ask the voice harness about a `\voice` dialogue block
    /// (`docui::voice`). `text` is an explicit utterance; without it
    /// the block's pending `you:` line is asked, or — in a
    /// `listen`/`full` block — the microphone is opened. `block` is
    /// the 0-based index among the document's `\voice` blocks; when
    /// absent the caret's block is used, else the first one. The
    /// answer arrives as a `voice_result` notification (or `error`).
    Voice {
        text: Option<String>,
        block: Option<usize>,
    },
    /// Add a DBus routing rule.
    DbusRouterAddRule {
        rule: emthin_dbus::router::RouteRule,
    },
    /// Remove a DBus routing rule by id.
    DbusRouterRemoveRule { id: String },
    /// List all current DBus routing rules.
    DbusRouterListRules,
    /// Ask for this document's current version token, hex encoded.
    CollabVersion,
    /// Join a collaborating peer at a version token. The answer is a
    /// `collab_delta` notification carrying everything the peer missed.
    /// `scope` is the caller's document generation; omitting it keeps the
    /// legacy unscoped path.
    CollabJoin {
        peer: String,
        version: String,
        scope: Option<u64>,
    },
    /// Apply a delta frame from a peer. `seq` is that peer's reliable-stream
    /// sequence number; `delta` is hex encoded. `epoch` is the connection
    /// epoch the peer learned from its `collab_join` reply: a frame from a
    /// replaced connection or a revoked epoch is refused instead of applied.
    CollabDelta {
        peer: String,
        seq: u64,
        delta: String,
        scope: Option<u64>,
        epoch: Option<u64>,
    },
    /// Acknowledge every frame through `seq` from `peer`. `epoch` is the
    /// connection epoch the peer learned from its `collab_join` reply; an ack
    /// from a replaced connection cannot advance a stream.
    CollabAck {
        peer: String,
        seq: u64,
        epoch: Option<u64>,
    },
    /// Drop a peer and its reliable stream.
    CollabLeave { peer: String },
    /// Apply one remote presence payload. `peer` is the transmitting peer and
    /// `blob` is hex encoded. Presence is advisory: it never edits the
    /// document.
    CollabPresence {
        peer: String,
        blob: String,
        scope: Option<u64>,
    },
    /// Set this process's collaboration display name.
    CollabName { name: String },
    /// Ask whether `peer` may discover this share. The reply is
    /// `collab_share_access`; a denied and an unknown identity both see
    /// `visible: false`.
    CollabShares { peer: String },
    /// Grant `read`/`edit`/`invite` on this share to `peer` (owner only).
    CollabGrant { peer: String, grant: String },
    /// Revoke all grants for `peer` and invalidate pending work (owner only).
    CollabRevoke { peer: String },
    /// Apply a share-scoped, anchored collaborator caret. `blob` is hex
    /// encoded; `epoch` fences a stale connection.
    CollabAnchor {
        peer: String,
        blob: String,
        epoch: Option<u64>,
    },
    /// List the retained document-history frontiers (hex).
    HistoryList,
    /// Preview a retained frontier without touching the live document.
    HistoryPreview { frontier: String },
    /// Explicit copy export: make a retained frontier the new local document.
    HistoryExport { frontier: String },
    /// Read-only semantic outline: definitions, uses and unresolved names.
    SemanticOutline,
    /// Go to the definition at a document byte (moves the caret).
    GotoDefinition { byte: usize },
    /// Rename a definition across its name and all resolved uses (one edit).
    RenameDefinition { def_index: usize, new_name: String },
    /// Read-only rendered-text accessibility nodes (opt-in query).
    A11yNodes,
    /// A second query over the proof view: bounded name search.
    ProofSearch { query: String, limit: usize },
    /// List dormant figures with a `launch:` command (the launcher palette).
    LauncherEntries,
    /// Launch a dormant figure by key, through the document's own `launch:`.
    LauncherLaunch { figure: String },
    /// Queue one notebook block for external kernel execution.
    KernelRun {
        block_end: usize,
        code: String,
        lang: Option<String>,
    },
    /// Read the last-good derived kernel output regions.
    KernelOutputs,
    /// Publish an AT Protocol session descriptor for the current document.
    CollabPublishDescriptor {
        owner: String,
        endpoint: String,
        rkey: String,
    },
    /// Fetch a published session descriptor (discovery only).
    CollabDescriptor { rkey: String },
    /// Queue one `\lint` diagnostics pass (third plugin).
    LintRun,
    /// List the helper-process plugins and their availability.
    PluginCatalog,
    /// Read the kernel-verified local control-client credentials.
    ClientIdentity,
    /// Enable opt-in bounded session diagnostics.
    InspectStart,
    /// Disable diagnostics and drop the event buffer.
    InspectStop,
    /// Report the settled state from the shared event reducer.
    InspectState,
    /// Build and return the read-only proof/reference view (K8).
    ProofQuery,
    /// Apply an independently fetched, read-only proof result. Ownership is
    /// explicit so a late result for another proof/document is ignored.
    ProofResult {
        share_id: String,
        document_key: String,
        proof_id: String,
        verdict: String,
    },
}

/// Contact phase for [`IncomingMessage::InjectTouch`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchPhase {
    Down,
    Motion,
    Up,
    Cancel,
}

impl TouchPhase {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "down" => Ok(Self::Down),
            "motion" => Ok(Self::Motion),
            "up" => Ok(Self::Up),
            "cancel" => Ok(Self::Cancel),
            other => Err(format!(
                "invalid touch phase '{other}' (expected down, motion, up or cancel)"
            )),
        }
    }
}

/// One accessible node on the wire: `(role, label, value, start, end)`. Byte
/// offsets are null when the node has no source range.
type A11yNodeRow = (String, String, Option<String>, Option<usize>, Option<usize>);

/// emthin → control client.
#[derive(Debug, Clone)]
pub enum OutgoingMessage {
    /// Sent once on connect; `version` is the protocol version.
    Connected {
        version: &'static str,
    },
    /// A figure changed shape or binding. `bound` is false for a
    /// dormant figure.
    FigureChanged {
        figure: String,
        page: usize,
        rect: IpcRect,
        bound: bool,
    },
    /// An app mapped into a figure.
    FigureBound {
        figure: String,
        window_id: u64,
        title: String,
    },
    /// An app unmapped or exited from a figure.
    FigureUnbound {
        figure: String,
        window_id: u64,
    },
    /// An app's title changed.
    AppTitleChanged {
        window_id: u64,
        title: String,
    },
    /// The visible page changed.
    PageChanged {
        page: usize,
    },
    /// One grouped, committed document change (K1). A presentation-only
    /// edit produces no notification, so this never advances a durable
    /// version on the client. `version` is the committed Loro version vector
    /// (hex); `ranges` are `[start, end]` UTF-8 byte pairs.
    DocChanged {
        seq: u64,
        document: u64,
        content_generation: u64,
        version: String,
        origin: String,
        scope: String,
        ranges: Vec<[usize; 2]>,
        source_digest: String,
        content_digest: String,
    },
    /// Response to `list_state`.
    State {
        page: usize,
        page_count: usize,
        /// Reading size in points.
        text_size: f32,
        /// Presentation zoom and pan, separate from the reading size.
        view_zoom: f64,
        view_pan_x: f64,
        view_pan_y: f64,
        citation_style: Box<Option<String>>,
        citation_locale: Box<Option<String>>,
        doc: Box<String>,
        figures: Vec<StateFigure>,
        /// Live peers, self excluded. Empty when no peer is connected.
        peers: Vec<StatePeer>,
        /// This process's stable presence id and display name.
        collab_local_peer: String,
        collab_local_name: String,
        /// The compact status line, or `null` in quiet state.
        status: Box<Option<String>>,
        /// Explicit-file status: `clean`, `unsaved`, or `failed`.
        save_state: &'static str,
        /// Recovery/session status: `clean`, `recovered`, or `failed`.
        recovery_state: &'static str,
        /// Preview status: `current`, `updating`, or `failed`.
        preview_state: &'static str,
        /// The full last layout failure, for a client that needs the detail.
        layout_error: Box<Option<String>>,
        /// The full explicit-file save/conflict failure, or `null`. Boxed so
        /// the already-large `State` variant does not grow the whole enum.
        save_error: Box<Option<String>>,
        /// The full recovery/session failure, or `null`.
        recovery_error: Box<Option<String>>,
        /// A non-fatal persistence recovery note, or `null`.
        save_warning: Box<Option<String>>,
        /// Local availability: `loading`, `available`, `missing`, `failed`.
        local_state: &'static str,
        /// Share readiness: `not_shared`, `catching_up`, `current`, `offline`,
        /// `refused`. Reported separately from the local document (K2).
        share_state: &'static str,
        local_state_detail: Box<Option<String>>,
        share_state_detail: Box<Option<String>>,
        /// Whether this document may seed a share.
        seed_granted: bool,
        /// The generation each kind of confirmation last covered:
        /// `(document, content_generation, version-hex)`. A peer ack confirms
        /// applied operations only, never disk.
        last_recovery_ack: Box<Option<(u64, u64, String)>>,
        last_explicit_ack: Box<Option<(u64, u64, String)>>,
        last_peer_ack: Box<Option<(u64, u64, String)>>,
    },
    /// The document was written to disk. `format` is `source` or `snapshot`;
    /// `digest` is the SHA-256 of the bytes actually written.
    DocSaved {
        path: String,
        format: String,
        digest: String,
    },
    /// The document's ASCII-only projection was written to disk.
    AsciiExported {
        path: String,
    },
    /// The reading size changed (or was clamped). `page_count` is the
    /// count after the reflow, so a client can confirm the effect without
    /// a second `list_state`.
    TextSizeChanged {
        pt: f32,
        page_count: usize,
    },
    /// A voice turn finished and was spliced into its `\voice` block
    /// as a `gemma:` line. `block` is the block's 0-based index at
    /// submit time; `utterance` is the question that was asked (typed,
    /// dictated, or passed to the `voice` method). A failed turn
    /// arrives as `Error` instead, and the failure is also spliced
    /// into the transcript as a `voice:` line.
    VoiceResult {
        block: usize,
        utterance: String,
        reply: String,
    },
    /// XWayland is ready; children can be spawned with `DISPLAY=:<n>`.
    XWaylandReady {
        display: u32,
    },
    /// Current DBus routing rules (response to `dbus_router_list_rules`).
    DbusRouterRules {
        rules: Vec<emthin_dbus::router::RouteRule>,
    },
    DbusRouterRuleAdded {
        id: String,
        rule: emthin_dbus::router::RouteRule,
    },
    DbusRouterRuleRemoved {
        id: String,
    },
    /// A collaboration frame for a peer. `delta` is hex encoded; `epoch` is
    /// the connection epoch `peer` must stamp on the frame it transports.
    CollabDelta {
        peer: String,
        seq: u64,
        delta: String,
        scope: Option<u64>,
        epoch: Option<u64>,
    },
    /// The document's current version token, hex encoded.
    CollabVersion {
        version: String,
    },
    /// The live peer snapshot and, when this tick published one, the local
    /// presence blob to relay. `blob` is hex encoded and may be forwarded as a
    /// `collab_presence` request; `peers` is derived, never durable.
    CollabPresence {
        local_peer: String,
        local_name: String,
        peers: Vec<StatePeer>,
        blob: Option<String>,
        scope: u64,
    },
    /// Reply to `collab_shares`. `visible` is false for both a denied and an
    /// unknown identity, so a client cannot enumerate shares (K4).
    CollabShareAccess {
        peer: String,
        visible: bool,
    },
    /// Reply to `semantic_outline`: definitions with use counts, resolved uses
    /// and unresolved names. Ranges are `[start,end]` UTF-8 byte pairs.
    SemanticOutline {
        definitions: Vec<(usize, String, usize, usize, usize)>,
        uses: Vec<(String, usize, usize, usize)>,
        unresolved: Vec<(String, usize, usize)>,
    },
    /// Reply to `rename_definition`: how many ranges changed, or an error.
    RenameResult {
        replacements: usize,
        applied: bool,
    },
    /// Reply to `client_identity`: the kernel-verified local peer, or null.
    ClientIdentity {
        pid: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
    },
    /// Reply to `lint_run`: whether a pass was accepted (a missing helper is
    /// a normal no-op).
    LintResult {
        generation: u64,
        accepted: bool,
    },
    /// Reply to `plugin_catalog`: `(name, available, runs)` triples for the
    /// helper-process plugins.
    PluginCatalog {
        plugins: Vec<(String, bool, usize)>,
    },
    /// Reply to `collab_publish_descriptor`: the committed record key and the
    /// checkpoint revision it advertised. Publication is best-effort.
    CollabDescriptorPublished {
        rkey: String,
        revision: u64,
    },
    /// Reply to `collab_descriptor`: the descriptor JSON, or null when the PDS
    /// has no such record. Discovery only — the live handshake still decides
    /// the real missing delta.
    CollabDescriptor {
        json: Option<String>,
    },
    /// Reply to `proof_search`: name matches as `(name,start,end)` rows plus
    /// unresolved names, sorted by source offset and bounded by the request.
    ProofMatches {
        definitions: Vec<(String, usize, usize)>,
        references: Vec<(String, usize, usize)>,
        missing: Vec<String>,
    },
    /// Reply to `kernel_run`: the generation fenced for this job, and whether
    /// it was accepted (a second concurrent run is rejected).
    KernelSubmitted {
        generation: u64,
        accepted: bool,
    },
    /// Reply to `kernel_outputs`: last-good derived regions as
    /// `[block_end, markup]` pairs. Never document text.
    KernelOutputs {
        regions: Vec<(usize, String)>,
    },
    /// Reply to `launcher_entries`: `(figure key, label, page)` triples.
    LauncherList {
        entries: Vec<(String, String, usize)>,
    },
    /// Reply to `launcher_launch`: whether the document's `launch:` was found
    /// and a spawn was attempted.
    LauncherLaunched {
        figure: String,
        spawned: bool,
    },
    /// Reply to `a11y_nodes`: read-only accessible nodes, each
    /// `{role,label,value,start,end}`. `start`/`end` are null when a node has
    /// no source range.
    A11yNodes {
        nodes: Vec<A11yNodeRow>,
    },
    /// Reply to `proof_query`: the read-only proof/reference projection.
    ProofView {
        share_id: String,
        document_key: String,
        revision: String,
        definitions: Vec<(String, usize, usize, usize)>,
        references: Vec<(String, usize, usize, Option<String>)>,
        missing: Vec<String>,
        proof_id: Option<String>,
        verdict: Option<String>,
    },
    /// Reply to `inspect_state`: the settled state from the shared reducer.
    InspectState {
        enabled: bool,
        local: String,
        share: String,
        frontier: String,
        accepted: u64,
        peers: usize,
        reason: Option<String>,
    },
    /// Reply to `history_list`: the retained frontiers, oldest first.
    History {
        frontiers: Vec<String>,
    },
    /// Reply to `history_preview`. `status` is `available` or `unavailable`;
    /// an unavailable preview carries a `reason` and no document content.
    /// A preview never changes the live model, save state or undo stack (K5).
    HistoryPreview {
        frontier: String,
        share_id: String,
        owner: String,
        seq: u64,
        text: String,
        marks: Vec<(usize, usize, String)>,
        content_digest: String,
        status: &'static str,
        reason: Option<String>,
    },
    /// A command failed. `code` is the machine-readable reason from the
    /// closed [`IpcErrorCode`] set; `message` is the human detail and is
    /// free to change wording at any time — clients must branch on `code`.
    Error {
        code: IpcErrorCode,
        message: String,
    },
}

/// The closed set of failure codes for the [`OutgoingMessage::Error`]
/// notification.
///
/// The wire carries `{code, message}`: a tagged error (contract shape
/// adapted from REA's error contract — code, category, retryability and
/// remediation are decidable without parsing prose). Each code's category,
/// retry semantics and remediation are documented in
/// `docs/error-contract.md`, which `the_documented_error_codes_are_the_only_
/// codes` pins to this enum in both directions — a code cannot ship without
/// its doc row, and a doc row cannot promise a code that does not exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcErrorCode {
    /// The figure exists in the document but no app is mapped into it.
    NotBound,
    /// The figure was bound to an app that has since exited.
    AppGone,
    /// No figure with that key exists in the document.
    UnknownFigure,
    /// A requested navigation target (e.g. a definition at a byte) was absent.
    NotFound,
    /// A voice turn could not be queued.
    VoiceFailed,
    /// A document file could not be read or written.
    IoFailed,
    /// `import_ascii` received a file containing non-ASCII bytes.
    NotAscii,
    /// A collaboration frame or version token could not be applied.
    CollabFailed,
    /// A document save would replace a file that changed since it was opened
    /// or last saved, and the request did not set `force`.
    DocConflict,
}

impl IpcErrorCode {
    /// The stable wire spelling; clients match on these strings.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotBound => "not_bound",
            Self::AppGone => "app_gone",
            Self::UnknownFigure => "unknown_figure",
            Self::NotFound => "not_found",
            Self::VoiceFailed => "voice_failed",
            Self::IoFailed => "io_failed",
            Self::NotAscii => "not_ascii",
            Self::CollabFailed => "collab_failed",
            Self::DocConflict => "doc_conflict",
        }
    }

    /// The failure category (closed set: `not_found`, `unavailable`,
    /// `execution_failure`, `io`, plus `malformed_input` for the inbound
    /// case that has no code). Sent on the wire so a client groups
    /// failures without maintaining its own code→category table.
    pub fn category(self) -> &'static str {
        match self {
            Self::NotBound | Self::UnknownFigure | Self::NotFound => "not_found",
            Self::AppGone => "unavailable",
            Self::VoiceFailed => "execution_failure",
            Self::IoFailed => "io",
            Self::NotAscii => "malformed_input",
            Self::CollabFailed => "malformed_input",
            Self::DocConflict => "io",
        }
    }

    /// Whether repeating the *same* request may succeed. Only
    /// `voice_failed` is retryable: the others are deterministic given
    /// the document/app state, and a client that retries them loops on
    /// the same refusal.
    pub fn retryable(self) -> bool {
        matches!(self, Self::VoiceFailed)
    }

    /// The documented recovery step, sent on the wire. Wording mirrors
    /// `docs/error-contract.md`'s remediation column; the pin test below
    /// keeps the two byte-equal per code, so a client can surface this
    /// string verbatim without fetching the contract.
    pub fn remediation(self) -> &'static str {
        match self {
            Self::NotBound => "The figure exists but no app is mapped into it. Spawn into it (or wait for `figure_bound`) before `close`/`focus`.",
            Self::AppGone => "The figure's app exited after binding. Re-`spawn`; the next `figure_bound` re-establishes the mapping.",
            Self::UnknownFigure => "No figure with that key exists — check `list_state`'s `figures` and the figure keys in the document.",
            Self::NotFound => "No target exists at that document position — check the byte and `semantic_outline` for definitions and uses.",
            Self::VoiceFailed => "The turn could not be queued (no `\\voice` block under the target, a pending harness error, …). The message carries the reason; fix the block and retry.",
            Self::IoFailed => "The document path could not be read/written — check the path exists and is writable by the compositor's user.",
            Self::NotAscii => "The file is not the ASCII interchange projection. Re-export it with `export_ascii`, or convert its non-ASCII characters, before importing.",
            Self::CollabFailed => "The collaboration frame or version token is malformed or does not belong to this document. Send a frame produced by the same document's `collab_version`/`collab_delta` exchange; do not reuse one peer's frame for another.",
            Self::DocConflict => "The file changed on disk since it was opened or last saved. Re-open it or save to a new path; repeat with `force: true` only to discard the outside change intentionally.",
        }
    }

    /// Every code, for the doc-sync test.
    pub const ALL: [IpcErrorCode; 9] = [
        Self::NotBound,
        Self::AppGone,
        Self::UnknownFigure,
        Self::NotFound,
        Self::VoiceFailed,
        Self::IoFailed,
        Self::NotAscii,
        Self::CollabFailed,
        Self::DocConflict,
    ];
}

/// One live collaborator as reported by `list_state` and `collab_presence`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StatePeer {
    /// Stable peer id published by that peer.
    pub peer: String,
    /// Display name published by that peer.
    pub name: String,
    /// Caret byte offset, already clamped to the local text length.
    pub cursor: Option<usize>,
    /// Whether the reliable delta stream is currently joined for this peer.
    pub connected: bool,
    /// Age of the last presence heartbeat in milliseconds.
    pub last_seen_age_ms: i64,
}

/// One figure as reported by `list_state`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StateFigure {
    /// The `\app` statement key (`f<stmt-index>`).
    pub key: String,
    /// The app binding id from the statement's 3rd argument.
    pub id: Option<String>,
    /// True when embedded inline inside a paragraph (`inline: true`).
    pub inline: bool,
    /// The figure's caption text.
    pub caption: String,
    /// Placed rect in output-local logical px (zero when off-page).
    pub rect: IpcRect,
    /// Page the figure is on.
    pub page: usize,
    /// `Some` when an app is bound.
    pub window_id: Option<u64>,
    /// The bound app's title.
    pub title: Option<String>,
}

// ---------------------------------------------------------------------------
// Manual JSON-RPC conversion (no serde derive)
// ---------------------------------------------------------------------------

impl IncomingMessage {
    pub fn from_jsonrpc(method: &str, params: &serde_json::Value) -> Result<Self, String> {
        Ok(match method {
            "spawn" => Self::Spawn {
                cmd: params_get_string(params, "cmd")?,
                args: params
                    .get("args")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
            },
            "close" => Self::Close {
                figure: params_get_string(params, "figure")?,
            },
            "focus" => Self::Focus {
                // `null` explicitly means "the document".
                figure: params
                    .get("figure")
                    .and_then(|v| v.as_str())
                    .map(String::from),
            },
            "set_figure_size" => Self::SetFigureSize {
                figure: params_get_string(params, "figure")?,
                w: params_get_i32(params, "w")?,
                h: params_get_i32(params, "h")?,
            },
            "clone_figure" => Self::CloneFigure {
                figure: params_get_string(params, "figure")?,
            },
            "goto_page" => Self::GotoPage {
                page: params_get_usize(params, "page")?,
            },
            "open_doc" => Self::OpenDoc {
                path: params_get_string(params, "path")?,
            },
            "save_doc" => {
                let format = match params.get("format") {
                    None | Some(serde_json::Value::Null) => None,
                    Some(serde_json::Value::String(value)) => Some(SaveFormat::parse(value)?),
                    Some(_) => return Err("save_doc: format must be a string".into()),
                };
                Self::SaveDoc {
                    path: params_get_string(params, "path")?,
                    format,
                    force: params_get_optional_bool(params, "force")?.unwrap_or(false),
                }
            }
            "export_ascii" => Self::ExportAscii {
                path: params_get_string(params, "path")?,
            },
            "import_ascii" => Self::ImportAscii {
                path: params_get_string(params, "path")?,
            },
            "set_text_size" => Self::SetTextSize {
                pt: params_get_f32(params, "pt")?,
            },
            "inject_touch" => Self::InjectTouch {
                phase: TouchPhase::parse(&params_get_string(params, "phase")?)?,
                slot: params_get_i32(params, "slot")?,
                x: params_get_f64(params, "x")?,
                y: params_get_f64(params, "y")?,
            },
            "list_state" => Self::ListState,
            // Both fields optional: omitted `text` means "ask the
            // pending you: line, or dictate", omitted `block` means
            // "the caret's voice block, else the first". Present-but-
            // wrong-typed fields are an error rather than a fallback:
            // a script that asked for text and silently got a
            // *recording* (or the wrong block) is exactly the kind of
            // surprise this protocol exists to avoid.
            "voice" => Self::Voice {
                text: params_get_optional_string(params, "text")?,
                block: params_get_optional_usize(params, "block")?,
            },
            "dbus_router_add_rule" => {
                let rule: emthin_dbus::router::RouteRule = serde_json::from_value(
                    params.get("rule").cloned().ok_or("missing field 'rule'")?,
                )
                .map_err(|e| format!("invalid rule: {e}"))?;
                Self::DbusRouterAddRule { rule }
            }
            "dbus_router_remove_rule" => Self::DbusRouterRemoveRule {
                id: params_get_string(params, "id")?,
            },
            "dbus_router_list_rules" => Self::DbusRouterListRules,
            "collab_version" => Self::CollabVersion,
            "collab_join" => Self::CollabJoin {
                peer: params_get_string(params, "peer")?,
                version: params_get_string(params, "version")?,
                scope: params_get_optional_u64(params, "scope")?,
            },
            "collab_delta" => Self::CollabDelta {
                peer: params_get_string(params, "peer")?,
                seq: params_get_u64(params, "seq")?,
                delta: params_get_string(params, "delta")?,
                scope: params_get_optional_u64(params, "scope")?,
                epoch: params_get_optional_u64(params, "epoch")?,
            },
            "collab_ack" => Self::CollabAck {
                peer: params_get_string(params, "peer")?,
                seq: params_get_u64(params, "seq")?,
                epoch: params_get_optional_u64(params, "epoch")?,
            },
            "collab_leave" => Self::CollabLeave {
                peer: params_get_string(params, "peer")?,
            },
            "collab_presence" => Self::CollabPresence {
                peer: params_get_string(params, "peer")?,
                blob: params_get_string(params, "blob")?,
                scope: params_get_optional_u64(params, "scope")?,
            },
            "collab_name" => Self::CollabName {
                name: params_get_string(params, "name")?,
            },
            "collab_shares" => Self::CollabShares {
                peer: params_get_string(params, "peer")?,
            },
            "collab_grant" => Self::CollabGrant {
                peer: params_get_string(params, "peer")?,
                grant: params_get_string(params, "grant")?,
            },
            "collab_revoke" => Self::CollabRevoke {
                peer: params_get_string(params, "peer")?,
            },
            "collab_anchor" => Self::CollabAnchor {
                peer: params_get_string(params, "peer")?,
                blob: params_get_string(params, "blob")?,
                epoch: params_get_optional_u64(params, "epoch")?,
            },
            "history_list" => Self::HistoryList,
            "history_preview" => Self::HistoryPreview {
                frontier: params_get_string(params, "frontier")?,
            },
            "history_export" => Self::HistoryExport {
                frontier: params_get_string(params, "frontier")?,
            },
            "semantic_outline" => Self::SemanticOutline,
            "goto_definition" => Self::GotoDefinition {
                byte: params_get_usize(params, "byte")?,
            },
            "rename_definition" => Self::RenameDefinition {
                def_index: params_get_usize(params, "def_index")?,
                new_name: params_get_string(params, "new_name")?,
            },
            "a11y_nodes" => Self::A11yNodes,
            "proof_search" => Self::ProofSearch {
                query: params_get_string(params, "query")?,
                limit: params_get_usize(params, "limit")?,
            },
            "launcher_entries" => Self::LauncherEntries,
            "launcher_launch" => Self::LauncherLaunch {
                figure: params_get_string(params, "figure")?,
            },
            "kernel_run" => Self::KernelRun {
                block_end: params_get_usize(params, "block_end")?,
                code: params_get_string(params, "code")?,
                lang: params_get_optional_string(params, "lang")?,
            },
            "kernel_outputs" => Self::KernelOutputs,
            "collab_publish_descriptor" => Self::CollabPublishDescriptor {
                owner: params_get_string(params, "owner")?,
                endpoint: params_get_string(params, "endpoint")?,
                rkey: params_get_string(params, "rkey")?,
            },
            "collab_descriptor" => Self::CollabDescriptor {
                rkey: params_get_string(params, "rkey")?,
            },
            "lint_run" => Self::LintRun,
            "plugin_catalog" => Self::PluginCatalog,
            "client_identity" => Self::ClientIdentity,
            "inspect_start" => Self::InspectStart,
            "inspect_stop" => Self::InspectStop,
            "inspect_state" => Self::InspectState,
            "proof_query" => Self::ProofQuery,
            "proof_result" => Self::ProofResult {
                share_id: params_get_string(params, "share_id")?,
                document_key: params_get_string(params, "document_key")?,
                proof_id: params_get_string(params, "proof_id")?,
                verdict: params_get_string(params, "verdict")?,
            },
            other => return Err(format!("unknown IPC method: {other}")),
        })
    }
}

fn params_get_string(params: &serde_json::Value, key: &str) -> Result<String, String> {
    params[key]
        .as_str()
        .map(String::from)
        .ok_or_else(|| format!("missing/invalid field '{key}'"))
}

fn params_get_i32(params: &serde_json::Value, key: &str) -> Result<i32, String> {
    params[key]
        .as_i64()
        .and_then(|v| i32::try_from(v).ok())
        .ok_or_else(|| format!("missing/invalid field '{key}'"))
}

fn params_get_f32(params: &serde_json::Value, key: &str) -> Result<f32, String> {
    params
        .get(key)
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite())
        .map(|v| v as f32)
        .ok_or_else(|| format!("missing or invalid field '{key}'"))
}

fn params_get_f64(params: &serde_json::Value, key: &str) -> Result<f64, String> {
    params
        .get(key)
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("missing or invalid field '{key}'"))
}

fn params_get_usize(params: &serde_json::Value, key: &str) -> Result<usize, String> {
    params[key]
        .as_u64()
        .and_then(|v| usize::try_from(v).ok())
        .ok_or_else(|| format!("missing/invalid field '{key}'"))
}

fn params_get_u64(params: &serde_json::Value, key: &str) -> Result<u64, String> {
    params[key]
        .as_u64()
        .ok_or_else(|| format!("missing/invalid field '{key}'"))
}

/// An optional boolean param: absent or `null` is `None`.
fn params_get_optional_bool(params: &serde_json::Value, key: &str) -> Result<Option<bool>, String> {
    match params.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(format!("{key} must be a boolean")),
    }
}

/// An optional string param: absent or `null` is `None`, anything
/// else must actually be a string.
fn params_get_optional_string(
    params: &serde_json::Value,
    key: &str,
) -> Result<Option<String>, String> {
    match params.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(v) => v
            .as_str()
            .map(|s| Some(s.to_string()))
            .ok_or_else(|| format!("invalid field '{key}' (expected a string)")),
    }
}

/// An optional non-negative integer param, same rules as
/// [`params_get_optional_string`].
fn params_get_optional_u64(params: &serde_json::Value, key: &str) -> Result<Option<u64>, String> {
    match params.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("field '{key}' must be an unsigned integer")),
    }
}

fn params_get_optional_usize(
    params: &serde_json::Value,
    key: &str,
) -> Result<Option<usize>, String> {
    match params.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .map(Some)
            .ok_or_else(|| format!("invalid field '{key}' (expected a non-negative integer)")),
    }
}

impl IpcRect {
    fn to_json(self) -> serde_json::Value {
        serde_json::json!({"x": self.x, "y": self.y, "w": self.w, "h": self.h})
    }
}

impl OutgoingMessage {
    pub fn method_name(&self) -> &'static str {
        match self {
            Self::Connected { .. } => "connected",
            Self::FigureChanged { .. } => "figure_changed",
            Self::FigureBound { .. } => "figure_bound",
            Self::FigureUnbound { .. } => "figure_unbound",
            Self::AppTitleChanged { .. } => "app_title_changed",
            Self::PageChanged { .. } => "page_changed",
            Self::DocChanged { .. } => "doc_changed",
            Self::State { .. } => "state",
            Self::DocSaved { .. } => "doc_saved",
            Self::AsciiExported { .. } => "ascii_exported",
            Self::TextSizeChanged { .. } => "text_size_changed",
            Self::VoiceResult { .. } => "voice_result",
            Self::XWaylandReady { .. } => "x_wayland_ready",
            Self::DbusRouterRules { .. } => "dbus_router_rules",
            Self::DbusRouterRuleAdded { .. } => "dbus_router_rule_added",
            Self::DbusRouterRuleRemoved { .. } => "dbus_router_rule_removed",
            Self::CollabDelta { .. } => "collab_delta",
            Self::CollabVersion { .. } => "collab_version",
            Self::CollabPresence { .. } => "collab_presence",
            Self::CollabShareAccess { .. } => "collab_share_access",
            Self::SemanticOutline { .. } => "semantic_outline",
            Self::RenameResult { .. } => "rename_result",
            Self::A11yNodes { .. } => "a11y_nodes",
            Self::LauncherList { .. } => "launcher_list",
            Self::LauncherLaunched { .. } => "launcher_launched",
            Self::KernelSubmitted { .. } => "kernel_submitted",
            Self::KernelOutputs { .. } => "kernel_outputs",
            Self::CollabDescriptorPublished { .. } => "collab_descriptor_published",
            Self::CollabDescriptor { .. } => "collab_descriptor",
            Self::LintResult { .. } => "lint_result",
            Self::PluginCatalog { .. } => "plugin_catalog",
            Self::ClientIdentity { .. } => "client_identity",
            Self::ProofMatches { .. } => "proof_matches",
            Self::ProofView { .. } => "proof_view",
            Self::InspectState { .. } => "inspect_state",
            Self::History { .. } => "history",
            Self::HistoryPreview { .. } => "history_preview",
            Self::Error { .. } => "error",
        }
    }

    pub fn into_params_value(self) -> serde_json::Value {
        match self {
            Self::Connected { version } => serde_json::json!({"version": version}),
            Self::FigureChanged {
                figure,
                page,
                rect,
                bound,
            } => serde_json::json!({
                "figure": figure,
                "page": page,
                "rect": rect.to_json(),
                "bound": bound,
            }),
            Self::FigureBound {
                figure,
                window_id,
                title,
            } => serde_json::json!({
                "figure": figure,
                "window_id": window_id,
                "title": title,
            }),
            Self::FigureUnbound { figure, window_id } => {
                serde_json::json!({"figure": figure, "window_id": window_id})
            }
            Self::AppTitleChanged { window_id, title } => {
                serde_json::json!({"window_id": window_id, "title": title})
            }
            Self::PageChanged { page } => serde_json::json!({"page": page}),
            Self::DocChanged {
                seq,
                document,
                content_generation,
                version,
                origin,
                scope,
                ranges,
                source_digest,
                content_digest,
            } => serde_json::json!({
                "seq": seq,
                "document": document,
                "content_generation": content_generation,
                "version": version,
                "origin": origin,
                "scope": scope,
                "ranges": ranges,
                "source_digest": source_digest,
                "content_digest": content_digest,
            }),
            Self::State {
                page,
                page_count,
                text_size,
                view_zoom,
                view_pan_x,
                view_pan_y,
                citation_style,
                citation_locale,
                doc,
                figures,
                peers,
                collab_local_peer,
                collab_local_name,
                status,
                save_state,
                recovery_state,
                preview_state,
                layout_error,
                save_error,
                recovery_error,
                save_warning,
                local_state,
                share_state,
                local_state_detail,
                share_state_detail,
                seed_granted,
                last_recovery_ack,
                last_explicit_ack,
                last_peer_ack,
            } => serde_json::json!({
                "page": page,
                "page_count": page_count,
                "text_size": text_size,
                "view_zoom": view_zoom,
                "view_pan_x": view_pan_x,
                "view_pan_y": view_pan_y,
                "citation_style": citation_style,
                "citation_locale": citation_locale,
                "doc": doc,
                "figures": figures,
                "peers": peers,
                "collab_local_peer": collab_local_peer,
                "collab_local_name": collab_local_name,
                "status": status,
                "save_state": save_state,
                "recovery_state": recovery_state,
                "preview_state": preview_state,
                "layout_error": layout_error,
                "save_error": save_error,
                "recovery_error": recovery_error,
                "save_warning": save_warning,
                "local_state": local_state,
                "share_state": share_state,
                "local_state_detail": local_state_detail,
                "share_state_detail": share_state_detail,
                "seed_granted": seed_granted,
                "last_recovery_ack": last_recovery_ack.map(|(document, generation, version)| {
                    serde_json::json!({"document": document, "content_generation": generation, "version": version})
                }),
                "last_explicit_ack": last_explicit_ack.map(|(document, generation, version)| {
                    serde_json::json!({"document": document, "content_generation": generation, "version": version})
                }),
                "last_peer_ack": last_peer_ack.map(|(document, generation, version)| {
                    serde_json::json!({"document": document, "content_generation": generation, "version": version})
                }),
            }),
            Self::DocSaved {
                path,
                format,
                digest,
            } => serde_json::json!({"path": path, "format": format, "digest": digest}),
            Self::AsciiExported { path } => serde_json::json!({"path": path}),
            Self::TextSizeChanged { pt, page_count } => {
                serde_json::json!({"pt": pt, "page_count": page_count})
            }
            Self::VoiceResult {
                block,
                utterance,
                reply,
            } => serde_json::json!({
                "block": block,
                "utterance": utterance,
                "reply": reply,
            }),
            Self::XWaylandReady { display } => serde_json::json!({"display": display}),
            Self::DbusRouterRules { rules } => {
                serde_json::json!({"rules": serde_json::to_value(rules).unwrap_or_default()})
            }
            Self::DbusRouterRuleAdded { id, rule } => serde_json::json!({
                "id": id,
                "rule": serde_json::to_value(rule).unwrap_or_default(),
            }),
            Self::DbusRouterRuleRemoved { id } => serde_json::json!({"id": id}),
            Self::CollabDelta {
                peer,
                seq,
                delta,
                scope,
                epoch,
            } => serde_json::json!({
                "peer": peer,
                "seq": seq,
                "delta": delta,
                "scope": scope,
                "epoch": epoch,
            }),
            Self::CollabVersion { version } => serde_json::json!({"version": version}),
            Self::CollabPresence {
                local_peer,
                local_name,
                peers,
                blob,
                scope,
            } => serde_json::json!({
                "local_peer": local_peer,
                "local_name": local_name,
                "peers": peers,
                "blob": blob,
                "scope": scope,
            }),
            Self::CollabShareAccess { peer, visible } => {
                serde_json::json!({"peer": peer, "visible": visible})
            }
            Self::SemanticOutline {
                definitions,
                uses,
                unresolved,
            } => serde_json::json!({
                "definitions": definitions,
                "uses": uses,
                "unresolved": unresolved,
            }),
            Self::RenameResult {
                replacements,
                applied,
            } => serde_json::json!({
                "replacements": replacements,
                "applied": applied,
            }),
            Self::A11yNodes { nodes } => serde_json::json!({ "nodes": nodes }),
            Self::ProofMatches {
                definitions,
                references,
                missing,
            } => serde_json::json!({
                "definitions": definitions,
                "references": references,
                "missing": missing,
            }),
            Self::KernelSubmitted {
                generation,
                accepted,
            } => {
                serde_json::json!({ "generation": generation, "accepted": accepted })
            }
            Self::KernelOutputs { regions } => serde_json::json!({ "regions": regions }),
            Self::CollabDescriptorPublished { rkey, revision } => {
                serde_json::json!({ "rkey": rkey, "revision": revision })
            }
            Self::CollabDescriptor { json } => serde_json::json!({ "json": json }),
            Self::LintResult {
                generation,
                accepted,
            } => {
                serde_json::json!({ "generation": generation, "accepted": accepted })
            }
            Self::PluginCatalog { plugins } => serde_json::json!({ "plugins": plugins }),
            Self::ClientIdentity { pid, uid, gid } => {
                serde_json::json!({ "pid": pid, "uid": uid, "gid": gid })
            }
            Self::LauncherList { entries } => serde_json::json!({ "entries": entries }),
            Self::LauncherLaunched { figure, spawned } => {
                serde_json::json!({ "figure": figure, "spawned": spawned })
            }
            Self::ProofView {
                share_id,
                document_key,
                revision,
                definitions,
                references,
                missing,
                proof_id,
                verdict,
            } => serde_json::json!({
                "share_id": share_id,
                "document_key": document_key,
                "revision": revision,
                "definitions": definitions,
                "references": references,
                "missing": missing,
                "proof_id": proof_id,
                "verdict": verdict,
            }),
            Self::InspectState {
                enabled,
                local,
                share,
                frontier,
                accepted,
                peers,
                reason,
            } => serde_json::json!({
                "enabled": enabled,
                "local": local,
                "share": share,
                "frontier": frontier,
                "accepted": accepted,
                "peers": peers,
                "reason": reason,
            }),
            Self::History { frontiers } => serde_json::json!({"frontiers": frontiers}),
            Self::HistoryPreview {
                frontier,
                share_id,
                owner,
                seq,
                text,
                marks,
                content_digest,
                status,
                reason,
            } => serde_json::json!({
                "frontier": frontier,
                "share_id": share_id,
                "owner": owner,
                "seq": seq,
                "text": text,
                "marks": marks,
                "content_digest": content_digest,
                "status": status,
                "reason": reason,
            }),
            // N2 (2026-10-13, REAdapt R16): the error notification now
            // carries `category`, `retryable` and `remediation` alongside
            // `code`/`message`. Additive — a client that parses only
            // code+message is unaffected; the fields let a client branch
            // and surface recovery text without fetching the contract doc.
            Self::Error { code, message } => {
                serde_json::json!({
                    "code": code.as_str(),
                    "message": message,
                    "category": code.category(),
                    "retryable": code.retryable(),
                    "remediation": code.remediation(),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_spawn_with_and_without_args() {
        let m = IncomingMessage::from_jsonrpc(
            "spawn",
            &serde_json::json!({"cmd": "foot", "args": ["-T", "xterm-256color"]}),
        )
        .expect("spawn");
        match m {
            IncomingMessage::Spawn { cmd, args } => {
                assert_eq!(cmd, "foot");
                assert_eq!(args, ["-T", "xterm-256color"]);
            }
            other => panic!("{other:?}"),
        }
        let m = IncomingMessage::from_jsonrpc("spawn", &serde_json::json!({"cmd": "foot"}))
            .expect("spawn");
        assert!(matches!(m, IncomingMessage::Spawn { ref args, .. } if args.is_empty()));
    }

    #[test]
    fn parses_focus_with_null_as_the_document() {
        let m = IncomingMessage::from_jsonrpc("focus", &serde_json::json!({"figure": null}))
            .expect("focus");
        assert!(matches!(m, IncomingMessage::Focus { figure: None }));
        let m = IncomingMessage::from_jsonrpc("focus", &serde_json::json!({"figure": "f2"}))
            .expect("focus");
        assert!(matches!(m, IncomingMessage::Focus { figure: Some(f) } if f == "f2"));
    }

    #[test]
    fn parses_set_figure_size_and_rejects_bad_numbers() {
        let m = IncomingMessage::from_jsonrpc(
            "set_figure_size",
            &serde_json::json!({"figure":"f0","w":320,"h":200}),
        )
        .expect("size");
        assert!(matches!(
            m,
            IncomingMessage::SetFigureSize { w: 320, h: 200, .. }
        ));
        // A float w is not an i32.
        assert!(IncomingMessage::from_jsonrpc(
            "set_figure_size",
            &serde_json::json!({"figure":"f0","w":1.5,"h":2})
        )
        .is_err());
    }

    #[test]
    fn parses_a_finite_text_size_and_rejects_missing_or_non_numeric_ones() {
        let m = IncomingMessage::from_jsonrpc("set_text_size", &serde_json::json!({"pt": 24.0}))
            .expect("set_text_size");
        assert!(
            matches!(m, IncomingMessage::SetTextSize { pt } if (pt - 24.0).abs() < f32::EPSILON)
        );
        assert!(IncomingMessage::from_jsonrpc("set_text_size", &serde_json::json!({})).is_err());
        assert!(
            IncomingMessage::from_jsonrpc("set_text_size", &serde_json::json!({"pt": "24"}))
                .is_err()
        );
    }

    #[test]
    fn parses_ascii_interchange_paths() {
        let m = IncomingMessage::from_jsonrpc(
            "export_ascii",
            &serde_json::json!({"path": "/tmp/out.typ"}),
        )
        .expect("export_ascii");
        assert!(matches!(m, IncomingMessage::ExportAscii { path } if path == "/tmp/out.typ"));
        let m = IncomingMessage::from_jsonrpc(
            "import_ascii",
            &serde_json::json!({"path": "/tmp/in.typ"}),
        )
        .expect("import_ascii");
        assert!(matches!(m, IncomingMessage::ImportAscii { path } if path == "/tmp/in.typ"));
        assert!(IncomingMessage::from_jsonrpc("export_ascii", &serde_json::json!({})).is_err());
        assert!(IncomingMessage::from_jsonrpc("import_ascii", &serde_json::json!({})).is_err());
    }

    #[test]
    fn parses_save_doc_format_and_force_and_rejects_bad_fields() {
        let m = IncomingMessage::from_jsonrpc(
            "save_doc",
            &serde_json::json!({"path":"/tmp/notes.mathed","format":"snapshot","force":true}),
        )
        .unwrap();
        assert!(matches!(
            m,
            IncomingMessage::SaveDoc {
                format: Some(SaveFormat::Snapshot),
                force: true,
                ..
            }
        ));

        let m = IncomingMessage::from_jsonrpc(
            "save_doc",
            &serde_json::json!({"path":"/tmp/notes.typ"}),
        )
        .unwrap();
        assert!(matches!(
            m,
            IncomingMessage::SaveDoc {
                format: None,
                force: false,
                ..
            }
        ));

        assert!(IncomingMessage::from_jsonrpc(
            "save_doc",
            &serde_json::json!({"path":"/tmp/notes.typ","format":"loro"})
        )
        .is_err());
        assert!(IncomingMessage::from_jsonrpc(
            "save_doc",
            &serde_json::json!({"path":"/tmp/notes.typ","force":"yes"})
        )
        .is_err());
        assert!(IncomingMessage::from_jsonrpc("save_doc", &serde_json::json!({})).is_err());
    }

    #[test]
    fn parses_page_navigation() {
        let m = IncomingMessage::from_jsonrpc("goto_page", &serde_json::json!({"page": 3}))
            .expect("goto_page");
        assert!(matches!(m, IncomingMessage::GotoPage { page: 3 }));
        assert!(
            IncomingMessage::from_jsonrpc("goto_page", &serde_json::json!({"page": -1})).is_err()
        );
    }

    #[test]
    fn parses_touch_injection_and_rejects_bad_phases_or_positions() {
        let m = IncomingMessage::from_jsonrpc(
            "inject_touch",
            &serde_json::json!({"phase":"down","slot":0,"x":10.5,"y":20}),
        )
        .expect("inject_touch");
        assert!(matches!(
            m,
            IncomingMessage::InjectTouch {
                phase: TouchPhase::Down,
                slot: 0,
                x,
                y,
            } if (x - 10.5).abs() < f64::EPSILON && (y - 20.0).abs() < f64::EPSILON
        ));
        assert!(IncomingMessage::from_jsonrpc(
            "inject_touch",
            &serde_json::json!({"phase":"sideways","slot":0,"x":1,"y":2})
        )
        .is_err());
        assert!(IncomingMessage::from_jsonrpc(
            "inject_touch",
            &serde_json::json!({"phase":"down","slot":0,"x":"1","y":2})
        )
        .is_err());
    }

    #[test]
    fn unknown_method_is_an_error_not_a_panic() {
        assert!(IncomingMessage::from_jsonrpc("nope", &serde_json::json!({})).is_err());
    }

    #[test]
    fn parses_the_collaboration_frames() {
        let m = IncomingMessage::from_jsonrpc(
            "collab_join",
            &serde_json::json!({"peer": "alice", "version": "0a"}),
        )
        .expect("join");
        assert!(matches!(m, IncomingMessage::CollabJoin { ref peer, .. } if peer == "alice"));

        let m = IncomingMessage::from_jsonrpc(
            "collab_delta",
            &serde_json::json!({"peer":"alice","seq":3,"delta":"beef"}),
        )
        .expect("delta");
        assert!(matches!(
            m,
            IncomingMessage::CollabDelta { seq: 3, ref peer, .. } if peer == "alice"
        ));

        let m = IncomingMessage::from_jsonrpc(
            "collab_ack",
            &serde_json::json!({"peer":"alice","seq":4}),
        )
        .expect("ack");
        assert!(matches!(m, IncomingMessage::CollabAck { seq: 4, .. }));

        let m = IncomingMessage::from_jsonrpc("collab_leave", &serde_json::json!({"peer":"alice"}))
            .expect("leave");
        assert!(matches!(m, IncomingMessage::CollabLeave { .. }));

        let m = IncomingMessage::from_jsonrpc(
            "collab_presence",
            &serde_json::json!({"peer":"alice","blob":"beef"}),
        )
        .expect("presence");
        assert!(matches!(
            m,
            IncomingMessage::CollabPresence { ref peer, ref blob, .. }
                if peer == "alice" && blob == "beef"
        ));

        let m = IncomingMessage::from_jsonrpc("collab_name", &serde_json::json!({"name":"Ada"}))
            .expect("name");
        assert!(matches!(m, IncomingMessage::CollabName { ref name } if name == "Ada"));

        assert!(matches!(
            IncomingMessage::from_jsonrpc("collab_version", &serde_json::json!({})).unwrap(),
            IncomingMessage::CollabVersion
        ));

        // A missing or negative sequence is not silently defaulted.
        assert!(IncomingMessage::from_jsonrpc(
            "collab_ack",
            &serde_json::json!({"peer":"alice","seq":-1})
        )
        .is_err());
    }

    #[test]
    fn collab_notifications_carry_hex_payloads() {
        let msg = OutgoingMessage::CollabDelta {
            peer: "alice".into(),
            seq: 2,
            delta: "00ff".into(),
            scope: Some(7),
            epoch: Some(3),
        };
        assert_eq!(msg.method_name(), "collab_delta");
        let json = msg.into_params_value();
        assert_eq!(json["peer"], "alice");
        assert_eq!(json["seq"], 2);
        assert_eq!(json["delta"], "00ff");
        assert_eq!(json["scope"], 7);
        assert_eq!(json["epoch"], 3);

        let msg = OutgoingMessage::CollabVersion {
            version: "0a0b".into(),
        };
        assert_eq!(msg.method_name(), "collab_version");
        assert_eq!(msg.into_params_value()["version"], "0a0b");

        let msg = OutgoingMessage::CollabPresence {
            local_peer: "a".into(),
            local_name: "Ada".into(),
            peers: vec![StatePeer {
                peer: "b".into(),
                name: "Bob".into(),
                cursor: Some(2),
                connected: true,
                last_seen_age_ms: 12,
            }],
            blob: Some("beef".into()),
            scope: 3,
        };
        assert_eq!(msg.method_name(), "collab_presence");
        let json = msg.into_params_value();
        assert_eq!(json["peers"][0]["name"], "Bob");
        assert_eq!(json["peers"][0]["cursor"], 2);
        assert_eq!(json["peers"][0]["connected"], true);
        assert_eq!(json["blob"], "beef");
    }

    #[test]
    fn doc_changed_carries_owner_origin_and_both_digests() {
        let msg = OutgoingMessage::DocChanged {
            seq: 4,
            document: 7,
            content_generation: 3,
            version: "0a0b".into(),
            origin: "undo".into(),
            scope: "content".into(),
            ranges: vec![[2, 8]],
            source_digest: "source".into(),
            content_digest: "content".into(),
        };
        assert_eq!(msg.method_name(), "doc_changed");
        let json = msg.into_params_value();
        assert_eq!(json["seq"], 4);
        assert_eq!(json["document"], 7);
        assert_eq!(json["version"], "0a0b");
        assert_eq!(json["origin"], "undo");
        assert_eq!(json["scope"], "content");
        assert_eq!(json["ranges"][0][0], 2);
        assert_eq!(json["ranges"][0][1], 8);
        assert_eq!(json["source_digest"], "source");
        assert_eq!(json["content_digest"], "content");
    }

    #[test]
    fn parses_voice_with_optional_text_and_block() {
        let m = IncomingMessage::from_jsonrpc("voice", &serde_json::json!({}))
            .expect("a bare voice turn");
        assert!(matches!(
            m,
            IncomingMessage::Voice {
                text: None,
                block: None
            }
        ));
        let m = IncomingMessage::from_jsonrpc(
            "voice",
            &serde_json::json!({"text": "hello", "block": 1}),
        )
        .expect("voice");
        match m {
            IncomingMessage::Voice { text, block } => {
                assert_eq!(text.as_deref(), Some("hello"));
                assert_eq!(block, Some(1));
            }
            other => panic!("{other:?}"),
        }
        // A non-string `text` is not silently ignored — it is an error
        // the caller can see, like every other malformed field.
        assert!(IncomingMessage::from_jsonrpc("voice", &serde_json::json!({"text": 4})).is_err());
    }

    #[test]
    fn every_outgoing_method_name_is_snake_case() {
        // Guards the manual wire spelling: the one thing a derive
        // would silently break.
        let msgs = [
            OutgoingMessage::Connected { version: "0.1" },
            OutgoingMessage::FigureChanged {
                figure: "f0".into(),
                page: 0,
                rect: IpcRect {
                    x: 0,
                    y: 0,
                    w: 1,
                    h: 1,
                },
                bound: true,
            },
            OutgoingMessage::FigureBound {
                figure: "f0".into(),
                window_id: 1,
                title: "t".into(),
            },
            OutgoingMessage::FigureUnbound {
                figure: "f0".into(),
                window_id: 1,
            },
            OutgoingMessage::AppTitleChanged {
                window_id: 1,
                title: "t".into(),
            },
            OutgoingMessage::PageChanged { page: 0 },
            OutgoingMessage::DocChanged {
                seq: 1,
                document: 7,
                content_generation: 3,
                version: "0a0b".into(),
                origin: "local".into(),
                scope: "content".into(),
                ranges: vec![[0, 5]],
                source_digest: "src".into(),
                content_digest: "content".into(),
            },
            OutgoingMessage::AsciiExported {
                path: "/tmp/out.typ".into(),
            },
            OutgoingMessage::TextSizeChanged {
                pt: 20.0,
                page_count: 3,
            },
            OutgoingMessage::VoiceResult {
                block: 0,
                utterance: "u".into(),
                reply: "r".into(),
            },
            OutgoingMessage::XWaylandReady { display: 1 },
            OutgoingMessage::DbusRouterRuleRemoved { id: "i".into() },
            OutgoingMessage::CollabDelta {
                peer: "a".into(),
                seq: 1,
                delta: "00".into(),
                scope: Some(1),
                epoch: Some(1),
            },
            OutgoingMessage::CollabVersion {
                version: "00".into(),
            },
            OutgoingMessage::CollabPresence {
                local_peer: "a".into(),
                local_name: "Ada".into(),
                peers: Vec::new(),
                blob: None,
                scope: 1,
            },
            OutgoingMessage::Error {
                code: IpcErrorCode::VoiceFailed,
                message: "m".into(),
            },
        ];
        for msg in msgs {
            let name = msg.method_name();
            assert!(
                name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{name} must be snake_case"
            );
        }
        // The historical quirk: XWaylandReady's wire name is
        // "x_wayland_ready", not "xwayland_ready".
        assert_eq!(
            OutgoingMessage::XWaylandReady { display: 1 }.method_name(),
            "x_wayland_ready"
        );
    }

    #[test]
    fn rect_round_trips_through_json() {
        let msg = OutgoingMessage::FigureChanged {
            figure: "f0".into(),
            page: 2,
            rect: IpcRect {
                x: 10,
                y: 20,
                w: 640,
                h: 400,
            },
            bound: false,
        };
        let json = msg.into_params_value();
        assert_eq!(json["rect"]["x"], 10);
        assert_eq!(json["rect"]["h"], 400);
        assert_eq!(json["page"], 2);
        assert_eq!(json["bound"], false);
    }

    #[test]
    fn the_documented_error_codes_are_the_only_codes() {
        // docs/error-contract.md is the contract; IpcErrorCode is the
        // implementation. Both directions are pinned: a code without a doc
        // row is undiscoverable, a doc row without a code is a lie.
        let doc =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/error-contract.md");
        let text = std::fs::read_to_string(&doc).expect(
            "docs/error-contract.md must exist — the error contract is part of the protocol",
        );
        let section = text
            .split("\n## Closed code set")
            .nth(1)
            .and_then(|s| s.split("\n## ").next())
            .expect("a '## Closed code set' section");
        let mut documented: Vec<String> = Vec::new();
        for line in section.lines() {
            // Rows look like: | `not_bound` | not_found | … |. The header row
            // (| `code` | …) is excluded by requiring the backtick.
            let mut cells = line.split('|');
            let (Some(_sep), Some(first), Some(_cat)) = (cells.next(), cells.next(), cells.next())
            else {
                continue;
            };
            let name = first.trim();
            if name.starts_with('`')
                && name.ends_with('`')
                && name.len() > 2
                && name[1..name.len() - 1]
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_')
            {
                documented.push(name[1..name.len() - 1].to_string());
            }
        }
        documented.sort();
        let mut implemented: Vec<String> = IpcErrorCode::ALL
            .iter()
            .map(|c| c.as_str().to_string())
            .collect();
        implemented.sort();
        assert_eq!(
            documented, implemented,
            "docs/error-contract.md's closed code set and IpcErrorCode must list exactly the same codes"
        );
        assert!(
            !documented.is_empty(),
            "the doc table parsed no codes — did the section heading move?"
        );
    }

    #[test]
    fn the_documented_error_semantics_match_the_wire_fields() {
        // N2 (2026-10-13): the error notification sends `category`,
        // `retryable` and `remediation` on the wire. Each must agree with
        // the doc table cell-for-cell — the doc is the contract (clients
        // read it), the enum methods are what actually ships. Both
        // directions: a doc edit without a code change fails, and a code
        // change without a doc edit fails. Proven non-vacuous in both
        // directions at authoring time (flip a cell → this fails; flip
        // the method → this fails).
        let doc =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/error-contract.md");
        let text = std::fs::read_to_string(&doc).expect("docs/error-contract.md must exist");
        let section = text
            .split("\n## Closed code set")
            .nth(1)
            .and_then(|s| s.split("\n## ").next())
            .expect("a '## Closed code set' section");
        let mut rows: Vec<(String, String, bool, String)> = Vec::new();
        for line in section.lines() {
            // | `code` | category | yes/no | remediation | raised-by | — five
            // cells after the leading separator; the header row's first cell
            // is backticked too but its second cell is "category", not a
            // category value, so the retry-cell check excludes it.
            let cells: Vec<&str> = line.split('|').collect();
            if cells.len() < 6 {
                continue;
            }
            let name = cells[1].trim();
            if !(name.starts_with('`') && name.ends_with('`') && name.len() > 2) {
                continue;
            }
            let code = name[1..name.len() - 1].to_string();
            let category = cells[2].trim().to_string();
            let retry_cell = cells[3].trim();
            let retryable = match retry_cell {
                "yes" => true,
                "no" => false,
                _ => continue, // header row
            };
            let remediation = cells[4].trim().to_string();
            assert!(
                !remediation.is_empty(),
                "{code}: empty remediation cell — every code ships recovery text"
            );
            rows.push((code, category, retryable, remediation));
        }
        assert_eq!(
            rows.len(),
            IpcErrorCode::ALL.len(),
            "doc table has {} semantic rows, IpcErrorCode has {} codes",
            rows.len(),
            IpcErrorCode::ALL.len()
        );
        for code in IpcErrorCode::ALL {
            let row = rows
                .iter()
                .find(|(c, ..)| c == code.as_str())
                .unwrap_or_else(|| panic!("no doc row for {}", code.as_str()));
            assert_eq!(
                row.1,
                code.category(),
                "{}: doc category vs wire category",
                code.as_str()
            );
            assert_eq!(
                row.2,
                code.retryable(),
                "{}: doc retry vs wire retryable",
                code.as_str()
            );
            assert_eq!(
                row.3,
                code.remediation(),
                "{}: doc remediation vs wire remediation (byte-equal)",
                code.as_str()
            );
        }
    }

    #[test]
    fn error_notification_carries_the_semantic_fields() {
        // The wire shape itself: a client parsing only code+message still
        // works (additive change), and the new fields are present with the
        // documented values.
        let msg = OutgoingMessage::Error {
            code: IpcErrorCode::VoiceFailed,
            message: "turn not queued".into(),
        };
        let json = msg.into_params_value();
        assert_eq!(json["code"], "voice_failed");
        assert_eq!(json["message"], "turn not queued");
        assert_eq!(json["category"], "execution_failure");
        assert_eq!(json["retryable"], true);
        assert!(json["remediation"].as_str().is_some_and(|s| !s.is_empty()));

        let msg = OutgoingMessage::Error {
            code: IpcErrorCode::UnknownFigure,
            message: "unknown figure f9".into(),
        };
        let json = msg.into_params_value();
        assert_eq!(json["retryable"], false);
        assert_eq!(json["category"], "not_found");
    }
}
