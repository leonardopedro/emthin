//! IBus: the input method GNOME ships, and therefore the default on most
//! desktops.
//!
//! # Why this exists
//!
//! The bridge only ever recognised `org.fcitx.*`, so on a stock GNOME session
//! every input-method message was forwarded verbatim and nothing was extracted:
//! no focus tracking, no cursor rectangle, no composition state. The IME feature
//! was dead by construction on the most common desktop, and installing fcitx5
//! changed nothing, because the host was running IBus.
//!
//! Measured on the machine this was written on:
//!
//! ```text
//! org.freedesktop.IBus   ACTIVATED
//! org.fcitx.Fcitx5       not on the bus
//! GTK_IM_MODULE=ibus     QT_IM_MODULE=ibus
//! ```
//!
//! # What did not need changing
//!
//! The transport. The broker forwards every interface it does not intercept to
//! the host bus and proxies name resolution, so an IBus client inside emthin can
//! already reach the host's IBus. Only the classifier was missing — this module
//! and one `if` in `router::bridge`.
//!
//! # Shape
//!
//! IBus's `InputContext` was modelled on fcitx5's, so the mapping is nearly
//! one-to-one. The differences that matter:
//!
//! * the cursor rectangle is `UpdateCursorRect(x, y, w, h)` — fcitx5 calls it
//!   `SetCursorRect` and has a `V2` with a scale factor; IBus has no scale, its
//!   coordinates are already surface-local;
//! * teardown is `Destroy` on the context, against fcitx5's `DestroyIC`;
//! * `CommitText` and `CommitPreedit` carry the text itself. emthin does **not**
//!   synthesise replies for these: the client's own `text-input-v3` commit path
//!   already delivers committed text, and duplicating it here would double it.
//!   They are classified so they can be recognised and logged, not so they can be
//!   answered.

use gio::prelude::ToVariant;
use gio::DBusMessage;

use crate::ime::ImeEvent;

pub const INPUT_METHOD_INTERFACE: &str = "org.freedesktop.IBus.InputMethod";
pub const INPUT_CONTEXT_INTERFACE: &str = "org.freedesktop.IBus.InputContext";

/// IBus is one well-known name. The fcitx list has three because fcitx publishes
/// a portal alias and a v4 name; IBus publishes none of those.
pub const IBUS_WELL_KNOWN_NAMES: &[&str] = &["org.freedesktop.IBus"];

/// The object path prefix IBus allocates input contexts under, e.g.
/// `/org/freedesktop/IBus/InputContext/_1`.
pub const INPUT_CONTEXT_PATH_PREFIX: &str = "/org/freedesktop/IBus/InputContext";

pub fn is_ibus_interface(iface: &str) -> bool {
    matches!(iface, INPUT_METHOD_INTERFACE | INPUT_CONTEXT_INTERFACE)
}

pub fn is_ibus_well_known(name: &str) -> bool {
    IBUS_WELL_KNOWN_NAMES.contains(&name)
}

/// A recognised IBus method call.
#[derive(Debug, Clone, PartialEq)]
pub enum IbusMethodCall {
    CreateInputContext {
        name: String,
    },
    FocusIn {
        ic_path: String,
    },
    FocusOut {
        ic_path: String,
    },
    UpdateCursorRect {
        ic_path: String,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    },
    Reset {
        ic_path: String,
    },
    Destroy {
        ic_path: String,
    },
    /// Recognised but deliberately not answered; see the module comment.
    CommitText {
        ic_path: String,
    },
    CommitPreedit {
        ic_path: String,
    },
}

/// Recognise a method call on one of IBus's interfaces.
pub fn classify(msg: &DBusMessage) -> Option<IbusMethodCall> {
    let iface = msg.interface()?;
    let member = msg.member()?;
    let path = msg.path()?;
    let body = msg.body();

    match (iface.as_str(), member.as_str()) {
        (INPUT_METHOD_INTERFACE, "CreateInputContext") => {
            // Signature: (s name) -> (o path). The name is advisory; the reply's
            // object path is what identifies the context.
            let name: String = msg
                .body()
                .and_then(|b| b.child_value(0).get())
                .unwrap_or_default();
            Some(IbusMethodCall::CreateInputContext { name })
        }
        (INPUT_CONTEXT_INTERFACE, "FocusIn") => Some(IbusMethodCall::FocusIn {
            ic_path: path.to_string(),
        }),
        (INPUT_CONTEXT_INTERFACE, "FocusOut") => Some(IbusMethodCall::FocusOut {
            ic_path: path.to_string(),
        }),
        (INPUT_CONTEXT_INTERFACE, "UpdateCursorRect") => {
            let (x, y, w, h): (i32, i32, i32, i32) = body?.get()?;
            Some(IbusMethodCall::UpdateCursorRect {
                ic_path: path.to_string(),
                x,
                y,
                w,
                h,
            })
        }
        (INPUT_CONTEXT_INTERFACE, "Reset") => Some(IbusMethodCall::Reset {
            ic_path: path.to_string(),
        }),
        (INPUT_CONTEXT_INTERFACE, "Destroy") => Some(IbusMethodCall::Destroy {
            ic_path: path.to_string(),
        }),
        (INPUT_CONTEXT_INTERFACE, "CommitText") => Some(IbusMethodCall::CommitText {
            ic_path: path.to_string(),
        }),
        (INPUT_CONTEXT_INTERFACE, "CommitPreedit") => Some(IbusMethodCall::CommitPreedit {
            ic_path: path.to_string(),
        }),
        _ => None,
    }
}

/// Lower a recognised call into the frontend-neutral event, or `None` for calls
/// emthin acts on rather than observes (reset, create, commit).
pub fn method_call_to_event(call: &IbusMethodCall) -> Option<ImeEvent> {
    match call {
        IbusMethodCall::FocusIn { ic_path } => Some(ImeEvent::FocusChanged {
            ic_path: ic_path.clone(),
            focused: true,
        }),
        IbusMethodCall::FocusOut { ic_path } => Some(ImeEvent::FocusChanged {
            ic_path: ic_path.clone(),
            focused: false,
        }),
        IbusMethodCall::UpdateCursorRect {
            ic_path,
            x,
            y,
            w,
            h,
        } => Some(ImeEvent::CursorRect {
            ic_path: ic_path.clone(),
            rect: [*x, *y, *w, *h],
        }),
        IbusMethodCall::Destroy { ic_path } => Some(ImeEvent::IcDestroyed {
            ic_path: ic_path.clone(),
        }),
        _ => None,
    }
}

/// Build the reply for the one call emthin must answer itself: `CreateInputContext`
/// must hand back an object path, because the client will use it for every
/// subsequent call and there is nothing to forward it to.
pub fn build_reply(msg: &DBusMessage, call: &IbusMethodCall) -> Option<DBusMessage> {
    match call {
        IbusMethodCall::CreateInputContext { name } => {
            let path = format!("{INPUT_CONTEXT_PATH_PREFIX}/{}", sanitise(name));
            let obj_path = glib::variant::ObjectPath::try_from(path.as_str()).ok()?;
            let reply = msg.new_method_reply();
            reply.set_body(&obj_path.to_variant());
            Some(reply)
        }
        // Everything else is either observed (and takes no reply) or forwarded.
        _ => None,
    }
}

/// An object path element: IBus would use `_1`, `_2`, ... but the reply is
/// synthesised here and never reaches the daemon, so the only requirement is
/// that it is a legal path element and stable for a given name.
fn sanitise(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let trimmed = cleaned.trim_matches('_');
    if trimmed.is_empty() {
        "default".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ibus_interfaces_and_names_are_recognised() {
        assert!(is_ibus_interface(INPUT_METHOD_INTERFACE));
        assert!(is_ibus_interface(INPUT_CONTEXT_INTERFACE));
        assert!(!is_ibus_interface("org.fcitx.Fcitx.InputContext1"));
        assert!(!is_ibus_interface("org.freedesktop.DBus"));
        assert!(is_ibus_well_known("org.freedesktop.IBus"));
        assert!(!is_ibus_well_known("org.fcitx.Fcitx5"));
    }

    #[test]
    fn focus_and_cursor_rect_lower_to_neutral_events() {
        let focus = IbusMethodCall::FocusIn {
            ic_path: "/ibus/ctx".into(),
        };
        assert_eq!(
            method_call_to_event(&focus),
            Some(ImeEvent::FocusChanged {
                ic_path: "/ibus/ctx".into(),
                focused: true
            })
        );

        let rect = IbusMethodCall::UpdateCursorRect {
            ic_path: "/ibus/ctx".into(),
            x: 10,
            y: 20,
            w: 1,
            h: 24,
        };
        assert_eq!(
            method_call_to_event(&rect),
            Some(ImeEvent::CursorRect {
                ic_path: "/ibus/ctx".into(),
                rect: [10, 20, 1, 24]
            })
        );

        let gone = IbusMethodCall::Destroy {
            ic_path: "/ibus/ctx".into(),
        };
        assert_eq!(
            method_call_to_event(&gone),
            Some(ImeEvent::IcDestroyed {
                ic_path: "/ibus/ctx".into()
            })
        );
    }

    /// Committed text is observed, not answered: the client's own text-input-v3
    /// path already delivers it, and replying here would double it.
    #[test]
    fn commits_are_recognised_but_produce_no_event_and_no_reply() {
        for call in [
            IbusMethodCall::CommitText {
                ic_path: "/c".into(),
            },
            IbusMethodCall::CommitPreedit {
                ic_path: "/c".into(),
            },
            IbusMethodCall::Reset {
                ic_path: "/c".into(),
            },
        ] {
            assert!(method_call_to_event(&call).is_none(), "{call:?}");
            assert!(build_reply(
                &DBusMessage::new_method_call(
                    Some("org.freedesktop.IBus"),
                    "/b",
                    Some(INPUT_CONTEXT_INTERFACE),
                    "FocusIn"
                ),
                &call
            )
            .is_none());
        }
    }

    #[test]
    fn object_path_elements_are_legal_and_stable() {
        assert_eq!(sanitise("foo"), "foo");
        assert_eq!(sanitise("my input!"), "my_input");
        assert_eq!(sanitise("///"), "default");
        assert_eq!(sanitise(""), "default");
        // A path element may not be empty or contain a slash.
        for name in ["a/b", "", "///", "with space"] {
            let s = sanitise(name);
            assert!(!s.is_empty(), "{name:?} produced an empty element");
            assert!(!s.contains('/'), "{name:?} produced {s:?}");
        }
    }
}

#[cfg(test)]
mod routing_tests {
    use super::*;
    use crate::fcitx;
    use crate::ibus;

    /// The regression this module exists to prevent: an IBus message must be
    /// *intercepted*, not forwarded. Before this arm existed every IBus call fell
    /// through to the upstream send path, so the bridge was a pipe and the IME
    /// feature was inert on GNOME.
    #[test]
    fn ibus_interfaces_are_not_mistaken_for_fcitx_or_for_passthrough() {
        for iface in [INPUT_METHOD_INTERFACE, INPUT_CONTEXT_INTERFACE] {
            assert!(
                ibus::is_ibus_interface(iface),
                "{iface} must be recognised as IBus"
            );
            assert!(
                !fcitx::is_fcitx_interface(iface),
                "{iface} must not be claimed by the fcitx frontend"
            );
            assert_ne!(
                iface, "org.freedesktop.DBus",
                "{iface} would be proxied rather than intercepted"
            );
        }
    }

    /// And the well-known name must be one the bridge knows, or it never learns
    /// which daemon owns the session bus.
    #[test]
    fn the_ibus_well_known_name_is_recognised() {
        assert!(ibus::is_ibus_well_known("org.freedesktop.IBus"));
        assert!(fcitx::is_fcitx_well_known("org.fcitx.Fcitx5"));
        // Neither frontend may claim the other's name.
        assert!(!fcitx::is_fcitx_well_known("org.freedesktop.IBus"));
        assert!(!ibus::is_ibus_well_known("org.fcitx.Fcitx5"));
    }

    /// Focus tracking is the whole point of the interception, so it must survive
    /// the lowering into the neutral event type.
    #[test]
    fn a_focus_in_out_cycle_produces_the_events_the_ime_bridge_consumes() {
        let path = "/org/freedesktop/IBus/InputContext/_7";
        let focus_in = ibus::classify(&DBusMessage::new_method_call(
            Some("org.freedesktop.IBus"),
            path,
            Some(INPUT_CONTEXT_INTERFACE),
            "FocusIn",
        ))
        .expect("FocusIn must be recognised");
        let focus_out = ibus::classify(&DBusMessage::new_method_call(
            Some("org.freedesktop.IBus"),
            path,
            Some(INPUT_CONTEXT_INTERFACE),
            "FocusOut",
        ))
        .expect("FocusOut must be recognised");

        let events: Vec<ImeEvent> = [&focus_in, &focus_out]
            .into_iter()
            .filter_map(ibus::method_call_to_event)
            .collect();
        assert_eq!(
            events,
            vec![
                ImeEvent::FocusChanged {
                    ic_path: path.into(),
                    focused: true
                },
                ImeEvent::FocusChanged {
                    ic_path: path.into(),
                    focused: false
                },
            ],
            "an IBus focus cycle must produce the same events an fcitx one does"
        );
    }
}
