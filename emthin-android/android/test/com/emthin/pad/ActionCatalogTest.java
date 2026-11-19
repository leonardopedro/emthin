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
        for (ActionCatalog.Id id : ActionCatalog.TOOLBAR) {
            check(seen.add(id), "toolbar action appears once: " + id);
            check(!ActionCatalog.label(id, false, false).isEmpty(),
                    "toolbar action has a label: " + id);
        }
        check(seen.contains(ActionCatalog.Id.SAVE), "Save is advertised");
        check(seen.contains(ActionCatalog.Id.MARKERS), "Markers is advertised");
        check(seen.contains(ActionCatalog.Id.CLOSE_INSPECTOR), "Close is advertised");

        check(ActionCatalog.label(ActionCatalog.Id.SAVE, false, false)
                        .equals("^S Save (Find: next)"),
                "toolbar Save explains the Find context");
        check(ActionCatalog.label(ActionCatalog.Id.SAVE, false, true)
                        .equals("^S Find next (Save: close Find)"),
                "toolbar Save flips while Find is open");
        check(ActionCatalog.label(ActionCatalog.Id.FIND_NEXT, false, true)
                        .equals("^S Next in Find"),
                "Find's next action says it is in Find");
        check(ActionCatalog.label(ActionCatalog.Id.MARKERS, true, false)
                        .equals("Tab Hide markers"),
                "Markers has dynamic visible state");
        check(ActionCatalog.label(ActionCatalog.Id.MARKERS, false, false)
                        .equals("Tab Show markers"),
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
