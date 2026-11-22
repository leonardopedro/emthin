package com.emthin.pad;

/** The pad's toolbar/keyboard action names and their visible bindings. */
public final class ActionCatalog {
    private ActionCatalog() {}

    public enum Id {
        NEW, UNDO, REDO, OPEN, SAVE, SAVE_COPY, FIND, FIND_REPLACE, FIND_NEXT,
        FIND_PREVIOUS, NAVIGATE_LAST, NAVIGATE_REFERENCES, NAVIGATE_SECTIONS,
        NAV_BACK, NAV_FORWARD, FOLD, MARKERS, READING_SMALLER, READING_LARGER,
        ZOOM_OUT, ZOOM_IN, CLOSE_INSPECTOR, SESSIONS,
        SHARE_OPEN, SHARE_JOIN, SHARE_LEAVE, SHARE_WATCH, PROOF
    }

    /** Toolbar order is the visible order; every entry also has a key binding. */
    public static final Id[] TOOLBAR = {
        Id.NEW, Id.UNDO, Id.REDO, Id.OPEN, Id.SAVE, Id.SAVE_COPY, Id.FIND,
        Id.NAVIGATE_LAST, Id.NAV_BACK, Id.NAV_FORWARD, Id.FOLD, Id.MARKERS,
        Id.READING_SMALLER, Id.READING_LARGER, Id.ZOOM_OUT, Id.ZOOM_IN,
        Id.CLOSE_INSPECTOR, Id.SESSIONS, Id.SHARE_OPEN, Id.SHARE_JOIN,
        Id.SHARE_LEAVE, Id.SHARE_WATCH, Id.PROOF
    };

    public static String label(Id id, boolean markersVisible, boolean findOpen) {
        switch (id) {
            case NEW: return "^N New";
            case UNDO: return "^Z Undo";
            case REDO: return "^Y Redo";
            case OPEN: return "^O Open";
            case SAVE: return findOpen
                    ? "^S Find next (Save: close Find)"
                    : "^S Save (Find: next)";
            case SAVE_COPY: return "^⇧S Copy";
            case FIND: return "^F Find";
            case FIND_REPLACE: return "^H Replace";
            case FIND_NEXT: return "^S Next in Find";
            case FIND_PREVIOUS: return "^R Previous";
            case NAVIGATE_LAST: return "^K Navigate";
            case NAVIGATE_REFERENCES: return "^K References";
            case NAVIGATE_SECTIONS: return "^G Sections";
            case NAV_BACK: return "Alt← Back";
            case NAV_FORWARD: return "Alt→ Forward";
            case FOLD: return "^L Fold";
            case MARKERS: return markersVisible ? "Tab Hide markers" : "Tab Show markers";
            case READING_SMALLER: return "A−";
            case READING_LARGER: return "A+";
            case ZOOM_OUT: return "Zoom−";
            case ZOOM_IN: return "Zoom+";
            case CLOSE_INSPECTOR: return "Esc Close";
            case SESSIONS: return "Sessions";
            case SHARE_OPEN: return "Share document";
            case SHARE_JOIN: return "Join share";
            case SHARE_LEAVE: return "Leave share";
            case SHARE_WATCH: return "Watch / Edit";
            case PROOF: return "MM0 proof";
            default: throw new IllegalArgumentException("unlabelled action " + id);
        }
    }

    /** Ctrl+S is context-sensitive: in Find it moves to the next match. */
    public static Id ctrlS(boolean findOpen, boolean shift) {
        if (shift) return Id.SAVE_COPY;
        return findOpen ? Id.FIND_NEXT : Id.SAVE;
    }
}
