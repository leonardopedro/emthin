// Input-method events, in the one shape every frontend produces.
//
// This started life as `fcitx::FcitxEvent`, which was a lie the moment there was
// a second frontend: nothing in the payload is fcitx-specific, and the consumers
// in `emthin::state::ime` care about *what happened* (focus moved, the cursor
// rectangle changed, the context went away), not which daemon said so.
//
// Each frontend has its own module — `fcitx`, `ibus` — with its own names and its
// own method signatures, and both lower into this. That is the whole reason the
// per-daemon delta is data rather than code: IBus's `InputContext` was modelled on
// fcitx5's, so the mapping is close to one-to-one, and where it is not the
// difference is a signature and a name.

/// What an input method told us. Lowered from each frontend's wire form.
#[derive(Debug, Clone, PartialEq)]
pub enum ImeEvent {
    /// An input context gained or lost keyboard focus.
    FocusChanged { ic_path: String, focused: bool },
    /// The text input position moved, in surface-local coordinates.
    CursorRect { ic_path: String, rect: [i32; 4] },
    /// The context is gone; any state keyed on `ic_path` must be dropped.
    IcDestroyed { ic_path: String },
}

impl ImeEvent {
    /// The input context this event is about.
    pub fn ic_path(&self) -> &str {
        match self {
            ImeEvent::FocusChanged { ic_path, .. }
            | ImeEvent::CursorRect { ic_path, .. }
            | ImeEvent::IcDestroyed { ic_path } => ic_path,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_context_is_reachable_without_matching_on_the_variant() {
        let ev = ImeEvent::CursorRect {
            ic_path: "/ibus/input/context".into(),
            rect: [1, 2, 3, 4],
        };
        assert_eq!(ev.ic_path(), "/ibus/input/context");
        assert_eq!(
            ImeEvent::IcDestroyed {
                ic_path: "/x".into()
            }
            .ic_path(),
            "/x"
        );
    }

    /// Both frontends lower into this type, which is the point: a consumer cannot
    /// tell which daemon produced an event, and must not be able to.
    #[test]
    fn events_from_either_frontend_are_the_same_type() {
        let from_fcitx = ImeEvent::FocusChanged {
            ic_path: "/a".into(),
            focused: true,
        };
        let from_ibus = ImeEvent::FocusChanged {
            ic_path: "/a".into(),
            focused: true,
        };
        assert_eq!(from_fcitx, from_ibus);
    }
}
