package com.emthin.pad;

import java.util.ArrayDeque;
import java.util.concurrent.Executor;

/** Host tests for bounded pages, stale search rejection and frozen scope. */
public final class FindControllerTest {
    private static int checks;

    private static void check(boolean condition, String message) {
        checks++;
        if (!condition) throw new AssertionError(message);
    }

    private static final class Direct implements Executor {
        @Override public void execute(Runnable command) { command.run(); }
    }

    private static final class Queue implements Executor {
        final ArrayDeque<Runnable> jobs = new ArrayDeque<>();
        @Override public void execute(Runnable command) { jobs.add(command); }
        void runAll() {
            while (!jobs.isEmpty()) jobs.removeFirst().run();
        }
    }

    private static final class FakeSearch implements FindController.SearchFn {
        final int count = 250;
        @Override public FindController.Page search(FindController.Request request) {
            if (request.query.isEmpty()) return FindController.Page.empty();
            if ("none".equals(request.query)) {
                return new FindController.Page(0, 0, 0, 0, 0,
                        new FindController.Match[0], null);
            }
            int total = "small".equals(request.query) ? 3 : count;
            int first = Math.min(total, request.anchor / 2);
            int startIndex = request.includeAll ? 0 : Math.max(0, request.pageStart);
            if (!request.includeAll && request.pageStart < 0) {
                startIndex = (first / FindController.PAGE_SIZE) * FindController.PAGE_SIZE;
            }
            int end = request.includeAll ? total
                    : Math.min(total, startIndex + FindController.PAGE_SIZE);
            FindController.Match[] matches = new FindController.Match[end - startIndex];
            for (int i = 0; i < matches.length; i++) {
                int at = (startIndex + i) * 2;
                matches[i] = new FindController.Match(at, at + 1, i % 7 == 0);
            }
            return new FindController.Page(total, total, 0, first, startIndex,
                    matches, null);
        }
    }

    private static FindController controller(FindController.SearchFn fn, Executor search,
                                             Executor callback, final int[] changes) {
        return new FindController(fn, search, callback, new FindController.Listener() {
            @Override public void onFindChanged() { changes[0]++; }
        });
    }

    public static void main(String[] args) {
        Direct direct = new Direct();
        int[] changes = {0};
        FindController find = controller(new FakeSearch(), direct, direct, changes);
        find.open("x x x", 0, 0, 0);
        find.queryChanged("x x x", "x");
        check(find.total() == 250, "the full count arrives with the first page");
        check(find.pageMatches().length == 200, "only one 200-entry page is retained");
        check(find.currentNumber() == 1, "the first match at/after origin is current");

        for (int i = 0; i < 199; i++) find.next("x x x");
        check(find.currentNumber() == 200, "navigation stays in the retained page");
        find.next("x x x");
        check(find.currentNumber() == 201, "next crosses the page boundary by request");
        check(find.pageMatches().length == 50, "the last page is bounded too");

        for (int i = 0; i < 50; i++) find.next("x x x");
        check(find.currentNumber() == 1 && find.status().startsWith("Wrapped"),
                "next wraps with an explicit cue");

        find.queryChanged("x x x", "none");
        check(find.total() == 0 && "No matches".equals(find.status()),
                "an empty result is not an error");
        find.queryChanged("x x x", "");
        check(find.total() == 0 && "".equals(find.status()),
                "an empty query clears highlights and counts");

        find.open("x x x", 4, 4, 4);
        check(!find.selectionAvailable(), "Selection scope is disabled without a selection");
        check(!find.setScope("x x x", FindController.SCOPE_SELECTION),
                "Selection scope cannot be chosen while disabled");
        find.open("x x x", 4, 2, 5);
        check(find.selectionAvailable(), "a non-empty selection is frozen on open");
        check(find.setScope("x x x", FindController.SCOPE_SELECTION),
                "Selection scope is available with a selection");
        check(find.scope() == FindController.SCOPE_SELECTION, "the chosen scope is retained");

        final Queue searches = new Queue();
        int[] staleChanges = {0};
        FindController stale = controller(new FakeSearch(), searches, direct, staleChanges);
        stale.open("x x x", 0, 0, 0);
        stale.queryChanged("x x x", "small");
        stale.queryChanged("x x x", "x");
        searches.runAll();
        check(stale.total() == 250, "a stale shorter result cannot replace the newest query");

        find.setScope("x x x", FindController.SCOPE_TEXT);
        find.queryChanged("x x x", "x");
        find.requestPlan("x x x", new FindController.PlanListener() {
            @Override public void onPlan(FindController.Page page, String error) {
                check(error == null, "plan request reports no error");
                check(page.total == 250 && page.matches.length == 250,
                        "Replace all receives one complete bounded plan");
            }
        });

        System.out.println("find controller tests passed (" + checks + " checks)");
    }
}
