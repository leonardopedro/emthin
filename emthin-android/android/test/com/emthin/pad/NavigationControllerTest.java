package com.emthin.pad;

public final class NavigationControllerTest {
    private static int checks;

    private static void check(boolean condition, String message) {
        checks++;
        if (!condition) throw new AssertionError(message);
    }

    private static NavigationController.Entry at(String kind, int start, int end) {
        return at(kind, start, end, false, false);
    }

    private static NavigationController.Entry at(String kind, int start, int end,
                                                 boolean inspector, boolean wholeDocument) {
        return new NavigationController.Entry(kind, kind, start, end,
                "", -1, start, "", -1, start, null, inspector, wholeDocument);
    }

    private static void historyTruncatesForwardAfterBack() {
        NavigationController nav = new NavigationController();
        nav.reset(at("origin", 0, 0));
        check(nav.push(at("search", 10, 12)), "first jump recorded");
        check(nav.push(at("section", 40, 40)), "second jump recorded");
        check(nav.size() == 3, "baseline plus two destinations");
        check(nav.back().start == 10, "back returns the middle destination");
        check(nav.forward().start == 40, "forward returns the later destination");
        check(nav.back() != null, "back to middle");
        check(nav.push(at("definition", 70, 70)), "new jump after Back");
        check(!nav.canForward(), "new destination truncates Forward");
        check(nav.current().start == 70, "new destination is current");
        check(nav.size() == 3, "truncated branch is gone");
        nav.replaceCurrent(at("definition", 75, 75));
        check(nav.current().start == 75, "current entry tracks the actual place");
        check(nav.back().start == 10, "earlier history is unchanged");
    }

    private static void adjacentDuplicatesDoNotFillHistory() {
        NavigationController nav = new NavigationController();
        nav.reset(at("origin", 0, 0));
        check(nav.push(at("search", 5, 8)), "first accepted search");
        check(!nav.push(at("search", 5, 8)), "same accepted search ignored");
        check(nav.size() == 2, "only baseline and one search entry");
        check(nav.push(at("search", 8, 10)), "a different range is a new entry");
        check(nav.size() == 3, "different search recorded");
    }

    private static void historyIsCappedAtThirty() {
        NavigationController nav = new NavigationController();
        nav.reset(at("origin", 0, 0));
        for (int i = 1; i <= 45; i++) nav.push(at("section", i * 10, i * 10));
        check(nav.size() == NavigationController.MAX_ENTRIES, "exactly 30 entries");
        check(nav.current().start == 450, "newest destination retained");
        check(nav.entries().get(0).start == 160, "oldest entries dropped");
        int backs = 0;
        while (nav.canBack()) { nav.back(); backs++; }
        check(backs == 29, "29 Back steps across the bounded list");
    }

    private static void inspectorScopeSurvivesBack() {
        NavigationController nav = new NavigationController();
        nav.reset(at("current", 15, 15, true, false));
        nav.push(at("diagnostic", 15, 21, true, true));
        check(nav.current().wholeDocument, "diagnostic source uses the whole document");
        NavigationController.Entry origin = nav.back();
        check(origin != null && origin.inspector, "Back still opens the source inspector");
        check(!origin.wholeDocument, "Back restores the modifier-at-caret scope");
    }

    private static void resetClearsCrossDocumentHistory() {
        NavigationController nav = new NavigationController();
        nav.reset(at("origin", 0, 0));
        nav.push(at("definition", 4, 4));
        nav.reset(at("origin", 90, 90));
        check(nav.size() == 1, "New/Open starts a fresh one-entry history");
        check(!nav.canBack() && !nav.canForward(), "no old document destinations");
        check(nav.current().start == 90, "current document baseline");
    }

    public static void main(String[] args) {
        historyTruncatesForwardAfterBack();
        adjacentDuplicatesDoNotFillHistory();
        historyIsCappedAtThirty();
        inspectorScopeSurvivesBack();
        resetClearsCrossDocumentHistory();
        System.out.println("navigation controller tests passed (" + checks + " checks)");
    }
}
