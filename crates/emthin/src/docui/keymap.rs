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
//! | `Ctrl+Shift+Return` | relaunch a dormant figure (the pointed-at one) |
//! | `PgUp` / `PgDn` | previous / next document page |
//! | `Home` / `End` | start / end of the document |
//! | `Ctrl+Home` / `Ctrl+End` | start / end of the line |
//! | `Ctrl+Shift+M` | clone the focused figure (add a mirror) |
//! | `Ctrl+Shift+V` | run a voice turn for the `\voice` block at the caret |
//! | `Escape` / `Ctrl+G` | cancel a math completion, close the find bar, else return focus to the document |
//! | `Ctrl++` / `Ctrl+-` | larger / smaller reading size (reflows the document) |
//! | `Ctrl+0` | reset the reading size |
//! | `Ctrl+Shift++` / `Ctrl+Shift+-` | zoom the view in / out |
//! | `Ctrl+Shift+0` | reset the view zoom and pan |
//! | `Ctrl+Shift+S` / `Ctrl+Shift+L` | cycle citation style / locale while a citation is open |
//! | `Ctrl+Shift+R` | reset the citation presentation to the document defaults |
//!
//! Figure-local bindings (resize with `Alt+arrows`) live in
//! `crate::grabs`' grab state, not here: they only apply while a figure
//! is focused.
//!
//! One binding is deliberately *not* in the table above: plain `Return` over
//! a dormant figure relaunches it instead of inserting a newline. It is not a
//! global binding, so it is resolved in `input::edit_document_key` against the
//! pointer position rather than classified here.
//!
//! The table used to describe `Ctrl+Shift+Return` as opening "the app launcher
//! (`spawn` prompt)". There is no prompt and never was: REWRITE_PLAN §9 lists
//! the spawn-launcher UX as a deliberate placeholder, and §5.10 requires only
//! that a dormant figure be relaunchable. What it needs `\app`'s `launch:`
//! argument for, which is a document concern, not a launcher's.

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
    /// Ask the voice harness about the `\voice` block at the caret —
    /// the typed `you:` line if there is one, else the microphone in a
    /// `listen`/`full` block. See `crate::docui::voice`.
    VoiceTurn,
    /// Drop keyboard focus from the figure back to the document caret.
    FocusDocument,
    /// `Ctrl+F`: open the find bar at the caret.
    Find,
    /// `F3` / `Shift+F3`: the next / previous match, selecting it.
    FindNext,
    FindPrevious,
    /// `Ctrl++` / `Ctrl+=`: next larger reading size.
    TextLarger,
    /// `Ctrl+-`: next smaller reading size.
    TextSmaller,
    /// `Ctrl+0`: reset the reading size.
    TextReset,
    /// `Ctrl+Shift++`: zoom the page view in.
    ViewLarger,
    /// `Ctrl+Shift+-`: zoom the page view out.
    ViewSmaller,
    /// `Ctrl+Shift+0`: reset view zoom and pan.
    ViewReset,
    /// `Alt+Arrow`: pan the zoomed page in that direction.
    ViewPanLeft,
    ViewPanRight,
    ViewPanUp,
    ViewPanDown,
    /// `Ctrl+Shift+S`: cycle the rendered citation style.
    CitationStyleNext,
    /// `Ctrl+Shift+L`: cycle the rendered citation locale.
    CitationLocaleNext,
    /// `Ctrl+Shift+R`: clear the citation presentation overrides.
    CitationReset,
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
        // Letter arms accept both cases: `sym` is the *modified* keysym
        // (shift applied), so a shift chord arrives uppercase — matching
        // only the lowercase symbol made `Ctrl+Shift+M` unreachable in
        // the real input path even though the unit test passed.
        keysyms::KEY_g | keysyms::KEY_G if ctrl => Some(Action::FocusDocument),
        keysyms::KEY_Home if ctrl => Some(Action::LineStart),
        keysyms::KEY_End if ctrl => Some(Action::LineEnd),
        keysyms::KEY_Home if !ctrl && !alt => Some(Action::DocumentStart),
        keysyms::KEY_End if !ctrl && !alt => Some(Action::DocumentEnd),
        keysyms::KEY_m | keysyms::KEY_M if ctrl && shift => Some(Action::CloneFigure),
        keysyms::KEY_v | keysyms::KEY_V if ctrl && shift => Some(Action::VoiceTurn),
        keysyms::KEY_f | keysyms::KEY_F if ctrl => Some(Action::Find),
        keysyms::KEY_F3 if shift => Some(Action::FindPrevious),
        keysyms::KEY_F3 => Some(Action::FindNext),
        // View zoom is the shifted form of the reading-size chord. Put it
        // before the plain `Ctrl` arms: `Ctrl+Shift+=` arrives as `plus`, and
        // `Ctrl+Shift+0` as `parenright` on the real keyboard.
        keysyms::KEY_plus | keysyms::KEY_equal | keysyms::KEY_KP_Add if ctrl && shift => {
            Some(Action::ViewLarger)
        }
        keysyms::KEY_underscore | keysyms::KEY_minus | keysyms::KEY_KP_Subtract
            if ctrl && shift =>
        {
            Some(Action::ViewSmaller)
        }
        keysyms::KEY_parenright | keysyms::KEY_0 | keysyms::KEY_KP_0 if ctrl && shift => {
            Some(Action::ViewReset)
        }
        // Arrows with Alt are view navigation; without it they move the caret.
        keysyms::KEY_Left if alt && !ctrl => Some(Action::ViewPanLeft),
        keysyms::KEY_Right if alt && !ctrl => Some(Action::ViewPanRight),
        keysyms::KEY_Up if alt && !ctrl => Some(Action::ViewPanUp),
        keysyms::KEY_Down if alt && !ctrl => Some(Action::ViewPanDown),
        keysyms::KEY_s | keysyms::KEY_S if ctrl && shift => Some(Action::CitationStyleNext),
        keysyms::KEY_l | keysyms::KEY_L if ctrl && shift => Some(Action::CitationLocaleNext),
        keysyms::KEY_r | keysyms::KEY_R if ctrl && shift => Some(Action::CitationReset),
        keysyms::KEY_plus | keysyms::KEY_equal | keysyms::KEY_KP_Add if ctrl => {
            Some(Action::TextLarger)
        }
        keysyms::KEY_minus | keysyms::KEY_KP_Subtract if ctrl => Some(Action::TextSmaller),
        keysyms::KEY_0 | keysyms::KEY_KP_0 if ctrl => Some(Action::TextReset),
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
        "Ctrl+Shift+V",
        "run a voice turn for the \\voice block at the caret",
    ),
    (
        "Escape / Ctrl+G",
        "cancel a math completion, close the find bar, else return focus from a figure to the document",
    ),
    ("Ctrl+F", "find in the document"),
    ("F3 / Shift+F3", "next / previous match"),
    ("Ctrl++ / Ctrl+-", "larger / smaller reading size"),
    ("Ctrl+0", "reset the reading size"),
    ("Ctrl+Shift++ / Ctrl+Shift+-", "zoom the view in / out"),
    ("Ctrl+Shift+0", "reset the view zoom and pan"),
    (
        "Ctrl+wheel / Ctrl+two-finger scroll",
        "zoom the view around the pointer",
    ),
    ("Alt+Arrows", "pan the zoomed page"),
    ("two-finger drag", "pan the zoomed page"),
    (
        "Ctrl+Shift+S / Ctrl+Shift+L",
        "cycle citation style / locale while a citation is open",
    ),
    (
        "Ctrl+Shift+R",
        "reset the citation presentation to the document defaults",
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
    fn find_is_ctrl_f_and_stepping_is_f3() {
        assert_eq!(
            classify(keysyms::KEY_f, KeyState::Pressed, true, false, false),
            Some(Action::Find)
        );
        assert_eq!(
            classify(keysyms::KEY_F, KeyState::Pressed, true, true, false),
            Some(Action::Find),
            "the real path delivers the modified keysym"
        );
        assert_eq!(
            classify(keysyms::KEY_F3, KeyState::Pressed, false, false, false),
            Some(Action::FindNext)
        );
        assert_eq!(
            classify(keysyms::KEY_F3, KeyState::Pressed, false, true, false),
            Some(Action::FindPrevious)
        );
        assert_eq!(
            classify(keysyms::KEY_f, KeyState::Pressed, false, false, false),
            None,
            "a plain f is text"
        );
    }

    #[test]
    fn reading_size_is_ctrl_plus_minus_and_zero() {
        assert_eq!(
            classify(keysyms::KEY_plus, KeyState::Pressed, true, false, false),
            Some(Action::TextLarger)
        );
        assert_eq!(
            classify(keysyms::KEY_equal, KeyState::Pressed, true, false, false),
            Some(Action::TextLarger),
            "the unshifted +/= key must work too"
        );
        assert_eq!(
            classify(keysyms::KEY_minus, KeyState::Pressed, true, false, false),
            Some(Action::TextSmaller)
        );
        assert_eq!(
            classify(keysyms::KEY_0, KeyState::Pressed, true, false, false),
            Some(Action::TextReset)
        );
        assert_eq!(
            classify(keysyms::KEY_equal, KeyState::Pressed, false, false, false),
            None,
            "plain = is text"
        );
    }

    #[test]
    fn shifted_reading_size_chords_are_view_zoom() {
        assert_eq!(
            classify(keysyms::KEY_plus, KeyState::Pressed, true, true, false),
            Some(Action::ViewLarger)
        );
        assert_eq!(
            classify(keysyms::KEY_underscore, KeyState::Pressed, true, true, false),
            Some(Action::ViewSmaller)
        );
        assert_eq!(
            classify(keysyms::KEY_parenright, KeyState::Pressed, true, true, false),
            Some(Action::ViewReset),
            "Ctrl+Shift+0 arrives as the modified `)` keysym on some layouts"
        );
        assert_eq!(
            classify(keysyms::KEY_0, KeyState::Pressed, true, true, false),
            Some(Action::ViewReset),
            "other layouts leave the digit unmodified"
        );
        assert_eq!(
            classify(keysyms::KEY_plus, KeyState::Pressed, true, false, false),
            Some(Action::TextLarger),
            "without shift the chord still changes reading size"
        );
    }

    #[test]
    fn alt_arrows_pan_and_plain_arrows_still_edit() {
        assert_eq!(
            classify(keysyms::KEY_Left, KeyState::Pressed, false, false, true),
            Some(Action::ViewPanLeft)
        );
        assert_eq!(
            classify(keysyms::KEY_Right, KeyState::Pressed, false, false, true),
            Some(Action::ViewPanRight)
        );
        assert_eq!(
            classify(keysyms::KEY_Up, KeyState::Pressed, false, false, true),
            Some(Action::ViewPanUp)
        );
        assert_eq!(
            classify(keysyms::KEY_Down, KeyState::Pressed, false, false, true),
            Some(Action::ViewPanDown)
        );
        assert_eq!(
            classify(keysyms::KEY_Left, KeyState::Pressed, false, false, false),
            None,
            "a bare arrow must fall through to caret movement"
        );
        assert_eq!(
            classify(keysyms::KEY_Left, KeyState::Pressed, true, false, true),
            None,
            "Ctrl+Alt+Left keeps its existing meaning"
        );
    }

    #[test]
    fn citation_presentation_shortcuts_are_ctrl_shift_letters() {
        assert_eq!(
            classify(keysyms::KEY_S, KeyState::Pressed, true, true, false),
            Some(Action::CitationStyleNext)
        );
        assert_eq!(
            classify(keysyms::KEY_L, KeyState::Pressed, true, true, false),
            Some(Action::CitationLocaleNext)
        );
        assert_eq!(
            classify(keysyms::KEY_R, KeyState::Pressed, true, true, false),
            Some(Action::CitationReset)
        );
        assert_eq!(
            classify(keysyms::KEY_s, KeyState::Pressed, true, false, false),
            None,
            "plain Ctrl+S is not a citation shortcut"
        );
    }

    #[test]
    fn every_documented_binding_is_reachable() {
        // A guard against the table drifting from the classifier.
        assert!(DOCUMENTED_KEYMAP
            .iter()
            .any(|(_, what)| what.contains("app launcher")));
        // The real input path feeds the *modified* keysym (shift
        // applied), so Ctrl+Shift+m arrives as uppercase `M` — pin that
        // path, not the lowercase state no shift chord can produce.
        assert_eq!(
            classify(keysyms::KEY_M, KeyState::Pressed, true, true, false),
            Some(Action::CloneFigure)
        );
    }

    #[test]
    fn the_voice_turn_needs_ctrl_and_shift_and_steals_nothing_from_paste() {
        assert_eq!(
            classify(keysyms::KEY_V, KeyState::Pressed, true, true, false),
            Some(Action::VoiceTurn),
            "the real input path delivers the modified (uppercase) keysym"
        );
        assert_eq!(
            classify(keysyms::KEY_v, KeyState::Pressed, true, false, false),
            None,
            "Ctrl+V is the document's paste — it must fall through"
        );
        assert_eq!(
            classify(keysyms::KEY_v, KeyState::Pressed, false, true, false),
            None,
            "Shift+V alone is plain text"
        );
    }
}
