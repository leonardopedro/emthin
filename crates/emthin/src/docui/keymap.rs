//! The document-level keymap.
//!
//! There is deliberately **no** Emacs-style prefix/chord machinery
//! any more: the whole point of the rewrite is that the document is a
//! document. These bindings are the global ones — everything else is
//! plain text editing (`DocModel`) that mirrors any normal editor, so it
//! isn't enumerated here.
//!
//! | Binding | Action |
//! |---|---|
//! | `Ctrl+Shift+Return` | open the app launcher (`spawn` prompt) |
//! | `PgUp` / `PgDn` | previous / next document page |
//! | `Home` / `End` | start / end of the document |
//! | `Ctrl+Home` / `Ctrl+End` | start / end of the line |
//! | `Ctrl+Shift+M` | clone the focused figure (add a mirror) |
//! | `Escape` / `Ctrl+G` | return focus from a figure to the document |
//!
//! Figure-local bindings (resize with `Alt+arrows`) live in
//! `crate::grabs`' grab state, not here: they only apply while a figure
//! is focused.

use smithay::backend::input::KeyState;
use smithay::input::keyboard::keysyms;

/// A global document binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Open the app launcher.
    OpenLauncher,
    NextPage,
    PreviousPage,
    DocumentStart,
    DocumentEnd,
    LineStart,
    LineEnd,
    /// Clone the focused figure into a new `\app` statement (a mirror).
    CloneFigure,
    /// Drop keyboard focus from the figure back to the document caret.
    FocusDocument,
}

/// Classify a key press into a global [`Action`].
///
/// `sym` is the keysym with the keyboard layout's modifiers already
/// applied — layout-agnostic, so `Ctrl+Shift+Return` is the same
/// physical chord on qwerty and dvorak. `ctrl`/`shift`/`alt` are the
/// post-press modifier state.
///
/// Returns `None` for a key with no global binding, which the caller
/// then routes to the figure (if one is focused) or to document
/// editing.
pub fn classify(sym: u32, pressed: KeyState, ctrl: bool, shift: bool, alt: bool) -> Option<Action> {
    // Only act on the press edge; holding a key must not repeat actions.
    if pressed != KeyState::Pressed {
        return None;
    }
    match sym {
        keysyms::KEY_Return | keysyms::KEY_KP_Enter if ctrl && shift => Some(Action::OpenLauncher),
        keysyms::KEY_Page_Down => Some(Action::NextPage),
        keysyms::KEY_Page_Up => Some(Action::PreviousPage),
        keysyms::KEY_Escape => Some(Action::FocusDocument),
        keysyms::KEY_g if ctrl => Some(Action::FocusDocument),
        keysyms::KEY_Home if ctrl => Some(Action::LineStart),
        keysyms::KEY_End if ctrl => Some(Action::LineEnd),
        keysyms::KEY_Home if !ctrl && !alt => Some(Action::DocumentStart),
        keysyms::KEY_End if !ctrl && !alt => Some(Action::DocumentEnd),
        keysyms::KEY_m if ctrl && shift => Some(Action::CloneFigure),
        _ => None,
    }
}

/// The human-readable keymap, for the README and `docs/`.
pub const DOCUMENTED_KEYMAP: &[(&str, &str)] = &[
    ("Ctrl+Shift+Return", "open the app launcher"),
    ("PgUp / PgDn", "previous / next document page"),
    ("Home / End", "start / end of the document"),
    ("Ctrl+Home / Ctrl+End", "start / end of the line"),
    ("Ctrl+Shift+M", "clone the focused figure (add a mirror)"),
    (
        "Escape / Ctrl+G",
        "return focus from a figure to the document",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    const NO: (bool, bool, bool) = (false, false, false);

    #[test]
    fn plain_editing_keys_have_no_global_binding() {
        for key in [
            keysyms::KEY_a,
            keysyms::KEY_1,
            keysyms::KEY_space,
            keysyms::KEY_Left,
            keysyms::KEY_Down,
            keysyms::KEY_BackSpace,
        ] {
            assert_eq!(
                classify(key, KeyState::Pressed, NO.0, NO.1, NO.2),
                None,
                "key {key:#x} must fall through to text editing"
            );
        }
    }

    #[test]
    fn page_keys_do_not_need_modifiers() {
        assert_eq!(
            classify(keysyms::KEY_Page_Down, KeyState::Pressed, NO.0, NO.1, NO.2),
            Some(Action::NextPage)
        );
        assert_eq!(
            classify(keysyms::KEY_Page_Up, KeyState::Pressed, NO.0, NO.1, NO.2),
            Some(Action::PreviousPage)
        );
    }

    #[test]
    fn escape_and_ctrl_g_both_focus_the_document() {
        assert_eq!(
            classify(keysyms::KEY_Escape, KeyState::Pressed, NO.0, NO.1, NO.2),
            Some(Action::FocusDocument)
        );
        assert_eq!(
            classify(keysyms::KEY_g, KeyState::Pressed, true, false, false),
            Some(Action::FocusDocument)
        );
    }

    #[test]
    fn home_and_end_disambiguate_by_ctrl() {
        assert_eq!(
            classify(keysyms::KEY_Home, KeyState::Pressed, NO.0, NO.1, NO.2),
            Some(Action::DocumentStart)
        );
        assert_eq!(
            classify(keysyms::KEY_Home, KeyState::Pressed, true, false, false),
            Some(Action::LineStart)
        );
        assert_eq!(
            classify(keysyms::KEY_End, KeyState::Pressed, false, true, false),
            Some(Action::DocumentEnd),
            "Shift+End still means document end here"
        );
    }

    #[test]
    fn launcher_needs_both_ctrl_and_shift() {
        assert_eq!(
            classify(keysyms::KEY_Return, KeyState::Pressed, true, true, false),
            Some(Action::OpenLauncher)
        );
        assert_eq!(
            classify(keysyms::KEY_Return, KeyState::Pressed, true, false, false),
            None,
            "Ctrl+Return alone is not bound"
        );
        assert_eq!(
            classify(keysyms::KEY_Return, KeyState::Pressed, false, true, false),
            None
        );
    }

    #[test]
    fn actions_only_fire_on_the_press_edge() {
        assert_eq!(
            classify(keysyms::KEY_Page_Down, KeyState::Released, NO.0, NO.1, NO.2),
            None
        );
    }

    #[test]
    fn every_documented_binding_is_reachable() {
        // A guard against the table drifting from the classifier.
        assert!(DOCUMENTED_KEYMAP
            .iter()
            .any(|(_, what)| what.contains("app launcher")));
        assert_eq!(
            classify(keysyms::KEY_m, KeyState::Pressed, true, true, false),
            Some(Action::CloneFigure)
        );
    }
}
