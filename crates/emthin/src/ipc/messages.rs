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
    /// Snapshot the current document to a path.
    SaveDoc { path: String },
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
}

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
    /// Response to `list_state`.
    State {
        page: usize,
        page_count: usize,
        doc: String,
        figures: Vec<StateFigure>,
    },
    /// The document was written to disk.
    DocSaved {
        path: String,
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
    /// A voice turn could not be queued.
    VoiceFailed,
    /// A document file could not be read or written.
    IoFailed,
}

impl IpcErrorCode {
    /// The stable wire spelling; clients match on these strings.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotBound => "not_bound",
            Self::AppGone => "app_gone",
            Self::UnknownFigure => "unknown_figure",
            Self::VoiceFailed => "voice_failed",
            Self::IoFailed => "io_failed",
        }
    }

    /// The failure category (closed set: `not_found`, `unavailable`,
    /// `execution_failure`, `io`, plus `malformed_input` for the inbound
    /// case that has no code). Sent on the wire so a client groups
    /// failures without maintaining its own code→category table.
    pub fn category(self) -> &'static str {
        match self {
            Self::NotBound | Self::UnknownFigure => "not_found",
            Self::AppGone => "unavailable",
            Self::VoiceFailed => "execution_failure",
            Self::IoFailed => "io",
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
            Self::VoiceFailed => "The turn could not be queued (no `\\voice` block under the target, a pending harness error, …). The message carries the reason; fix the block and retry.",
            Self::IoFailed => "The document path could not be read/written — check the path exists and is writable by the compositor's user.",
        }
    }

    /// Every code, for the doc-sync test.
    pub const ALL: [IpcErrorCode; 5] = [
        Self::NotBound,
        Self::AppGone,
        Self::UnknownFigure,
        Self::VoiceFailed,
        Self::IoFailed,
    ];
}

/// One figure as reported by `list_state`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StateFigure {
    /// The `\app` statement key (`f<stmt-index>`).
    pub key: String,
    /// The app binding id from the statement's 3rd argument.
    pub id: Option<String>,
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
            "save_doc" => Self::SaveDoc {
                path: params_get_string(params, "path")?,
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

fn params_get_usize(params: &serde_json::Value, key: &str) -> Result<usize, String> {
    params[key]
        .as_u64()
        .and_then(|v| usize::try_from(v).ok())
        .ok_or_else(|| format!("missing/invalid field '{key}'"))
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
            Self::State { .. } => "state",
            Self::DocSaved { .. } => "doc_saved",
            Self::VoiceResult { .. } => "voice_result",
            Self::XWaylandReady { .. } => "x_wayland_ready",
            Self::DbusRouterRules { .. } => "dbus_router_rules",
            Self::DbusRouterRuleAdded { .. } => "dbus_router_rule_added",
            Self::DbusRouterRuleRemoved { .. } => "dbus_router_rule_removed",
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
            Self::State {
                page,
                page_count,
                doc,
                figures,
            } => serde_json::json!({
                "page": page,
                "page_count": page_count,
                "doc": doc,
                "figures": figures,
            }),
            Self::DocSaved { path } => serde_json::json!({"path": path}),
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
    fn parses_page_navigation() {
        let m = IncomingMessage::from_jsonrpc("goto_page", &serde_json::json!({"page": 3}))
            .expect("goto_page");
        assert!(matches!(m, IncomingMessage::GotoPage { page: 3 }));
        assert!(
            IncomingMessage::from_jsonrpc("goto_page", &serde_json::json!({"page": -1})).is_err()
        );
    }

    #[test]
    fn unknown_method_is_an_error_not_a_panic() {
        assert!(IncomingMessage::from_jsonrpc("nope", &serde_json::json!({})).is_err());
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
            OutgoingMessage::VoiceResult {
                block: 0,
                utterance: "u".into(),
                reply: "r".into(),
            },
            OutgoingMessage::XWaylandReady { display: 1 },
            OutgoingMessage::DbusRouterRuleRemoved { id: "i".into() },
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
