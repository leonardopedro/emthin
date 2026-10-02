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
    /// A command failed.
    Error {
        message: String,
    },
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
            Self::XWaylandReady { display } => serde_json::json!({"display": display}),
            Self::DbusRouterRules { rules } => {
                serde_json::json!({"rules": serde_json::to_value(rules).unwrap_or_default()})
            }
            Self::DbusRouterRuleAdded { id, rule } => serde_json::json!({
                "id": id,
                "rule": serde_json::to_value(rule).unwrap_or_default(),
            }),
            Self::DbusRouterRuleRemoved { id } => serde_json::json!({"id": id}),
            Self::Error { message } => serde_json::json!({"message": message}),
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
            OutgoingMessage::XWaylandReady { display: 1 },
            OutgoingMessage::DbusRouterRuleRemoved { id: "i".into() },
            OutgoingMessage::Error {
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
}
