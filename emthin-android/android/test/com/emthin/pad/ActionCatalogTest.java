package com.emthin.pad;

import java.util.HashSet;
import java.util.Set;

public final class ActionCatalogTest {
    private static int checks;

    private static void check(boolean condition, String message) {
        checks++;
        if (!condition) throw new AssertionError(message);
    }

    public static void main(String[] args) {
        Set<ActionCatalog.Id> seen = new HashSet<>();
        ActionCatalog.Id[][] groups = {
            ActionCatalog.TOOLBAR, ActionCatalog.DOCUMENT_MENU, ActionCatalog.NAVIGATION_MENU,
            ActionCatalog.VIEW_MENU, ActionCatalog.SESSIONS_MENU
        };
        for (ActionCatalog.Id[] group : groups) {
            for (ActionCatalog.Id id : group) {
                check(seen.add(id), "action appears once: " + id);
                check(!ActionCatalog.label(id, false).isEmpty(), "action has a label: " + id);
            }
        }
        check(java.util.Arrays.equals(ActionCatalog.TOOLBAR, new ActionCatalog.Id[] {
                ActionCatalog.Id.OPEN, ActionCatalog.Id.SAVE, ActionCatalog.Id.UNDO,
                ActionCatalog.Id.FIND, ActionCatalog.Id.EDIT_SOURCE
        }), "primary editing actions fit in one toolbar");
        check(seen.contains(ActionCatalog.Id.MARKERS), "Markers is advertised");
        check(seen.contains(ActionCatalog.Id.CLOSE_INSPECTOR), "Close is advertised");

        check(ActionCatalog.label(ActionCatalog.Id.SAVE, false).equals("Save"),
                "toolbar Save names what the button does");
        check(ActionCatalog.label(ActionCatalog.Id.FIND_NEXT, false).equals("Next"),
                "Find names what its next button does");
        check(ActionCatalog.label(ActionCatalog.Id.MARKERS, true)
                        .equals("Hide markers"),
                "Markers has dynamic visible state");
        check(ActionCatalog.label(ActionCatalog.Id.MARKERS, false)
                        .equals("Show markers"),
                "Markers has the hidden state label");

        check(ActionCatalog.ctrlS(false, false) == ActionCatalog.Id.SAVE,
                "Ctrl+S saves outside Find");
        check(ActionCatalog.ctrlS(true, false) == ActionCatalog.Id.FIND_NEXT,
                "Ctrl+S finds next inside Find");
        check(ActionCatalog.ctrlS(false, true) == ActionCatalog.Id.SAVE_COPY,
                "Ctrl+Shift+S saves a copy outside Find");
        check(ActionCatalog.ctrlS(true, true) == ActionCatalog.Id.SAVE_COPY,
                "Ctrl+Shift+S still saves a copy inside Find");

        System.out.println("action catalog tests passed (" + checks + " checks)");
    }
}
