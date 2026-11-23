package com.emthin.pad;

/** The pad's toolbar/keyboard action names and their visible bindings. */
public final class ActionCatalog {
    private ActionCatalog() {}

    public enum Id {
        NEW, UNDO, REDO, OPEN, SAVE, SAVE_COPY, EDIT_SOURCE, FIND, FIND_REPLACE, FIND_NEXT,
        FIND_PREVIOUS, NAVIGATE_LAST, NAVIGATE_REFERENCES, NAVIGATE_SECTIONS,
        NAV_BACK, NAV_FORWARD, FOLD, MARKERS, READING_SMALLER, READING_LARGER,
        ZOOM_OUT, ZOOM_IN, CLOSE_INSPECTOR, SESSIONS,
        SHARE_OPEN, SHARE_JOIN, SHARE_LEAVE, SHARE_WATCH, PROOF, HISTORY, DIAGNOSTICS,
        PAIR
    }

    public static final Id[] TOOLBAR = {Id.OPEN, Id.SAVE, Id.UNDO, Id.FIND, Id.EDIT_SOURCE};
    public static final Id[] DOCUMENT_MENU = {Id.NEW, Id.REDO, Id.SAVE_COPY, Id.HISTORY};
    public static final Id[] NAVIGATION_MENU = {Id.NAVIGATE_LAST, Id.NAV_BACK, Id.NAV_FORWARD};
    public static final Id[] VIEW_MENU = {
        Id.FOLD, Id.MARKERS, Id.DIAGNOSTICS,
        Id.READING_SMALLER, Id.READING_LARGER, Id.ZOOM_OUT, Id.ZOOM_IN,
        Id.CLOSE_INSPECTOR
    };
    public static final Id[] SESSIONS_MENU = {
        Id.SESSIONS, Id.PAIR, Id.SHARE_OPEN, Id.SHARE_JOIN, Id.SHARE_LEAVE, Id.SHARE_WATCH, Id.PROOF
    };

    public static String label(Id id, boolean markersVisible) {
        switch (id) {
            case NEW: return "New";
            case UNDO: return "Undo";
            case REDO: return "Redo";
            case OPEN: return "Open";
            case SAVE: return "Save";
            case SAVE_COPY: return "Save a copy";
            case EDIT_SOURCE: return "Edit";
            case FIND: return "Find";
            case FIND_REPLACE: return "Replace";
            case FIND_NEXT: return "Next";
            case FIND_PREVIOUS: return "Previous";
            case NAVIGATE_LAST: return "Go to…";
            case NAVIGATE_REFERENCES: return "References";
            case NAVIGATE_SECTIONS: return "Sections";
            case NAV_BACK: return "Back";
            case NAV_FORWARD: return "Forward";
            case FOLD: return "Fold or unfold";
            case MARKERS: return markersVisible ? "Hide markers" : "Show markers";
            case READING_SMALLER: return "Smaller text";
            case READING_LARGER: return "Larger text";
            case ZOOM_OUT: return "Zoom out";
            case ZOOM_IN: return "Zoom in";
            case CLOSE_INSPECTOR: return "Show document";
            case SESSIONS: return "Sessions";
            case PAIR: return "Pair gateway";
            case SHARE_OPEN: return "Share document";
            case SHARE_JOIN: return "Join share";
            case SHARE_LEAVE: return "Leave share";
            case SHARE_WATCH: return "Watch / Edit";
            case PROOF: return "MM0 proof";
            case HISTORY: return "History";
            case DIAGNOSTICS: return "Diagnostics";
            default: throw new IllegalArgumentException("unlabelled action " + id);
        }
    }

    /** Ctrl+S is context-sensitive: in Find it moves to the next match. */
    public static Id ctrlS(boolean findOpen, boolean shift) {
        if (shift) return Id.SAVE_COPY;
        return findOpen ? Id.FIND_NEXT : Id.SAVE;
    }
}
