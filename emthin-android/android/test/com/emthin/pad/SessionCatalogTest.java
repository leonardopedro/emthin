package com.emthin.pad;

import java.util.List;

public final class SessionCatalogTest {
    private static int checks;

    private static void check(boolean condition, String message) {
        checks++;
        if (!condition) throw new AssertionError(message);
    }

    public static void main(String[] args) {
        SessionCatalog catalog = SessionCatalog.fixture();
        check(catalog.entries().size() == 4, "fixture catalogue has four sessions");
        check(catalog.hosts().size() == 3, "and the hosts they came from");
        SessionCatalog.Entry live = catalog.byReference(
                "hc://6ab09a20fd1288983dfbbac2bb349231/t3");
        check(live != null && live.state == SessionCatalog.State.LIVE, "live entry");
        check(live.canPin(), "live session can be pinned");
        check("Pin to document".equals(SessionCatalog.actionLabel("PIN", live)),
                "pin label");
        check(catalog.pinned().size() == 1, "one pinned entry");
        SessionCatalog.Entry watched = catalog.byReference(
                "hc://0123456789abcdef0123456789abcdef/t7");
        check(watched.canDetach(), "watched session can detach");
        check("Detach".equals(SessionCatalog.actionLabel("DETACH", watched)),
                "detach label");
        SessionCatalog.Entry legacy = catalog.byReference(
                "herdr-local://fedcba9876543210fedcba9876543210/pane%20one");
        check(!legacy.canPin(), "unavailable session cannot pin");
        SessionCatalog.Entry offline = catalog.byReference(
                "hc://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/t9");
        check(offline != null && offline.state == SessionCatalog.State.OFFLINE,
                "offline entry");
        check(!offline.canPin(), "offline session cannot pin");
        SessionCatalog detached = catalog.detach(watched.reference);
        SessionCatalog.Entry unwatched = detached.byReference(watched.reference);
        check(unwatched != null && unwatched.state == SessionCatalog.State.LIVE,
                "detach releases the watcher but keeps the session");
        check(SessionCatalog.empty().hosts().isEmpty(), "an empty catalogue has no hosts");
        check(!unwatched.canDetach(), "detached session has no watcher");
        check(detached.pin(watched.reference)
                .byReference(watched.reference).pinned, "pin updates the live entry");
        check(catalog.without(live.reference).byReference(live.reference) == null,
                "forget removes the lease");
        check(catalog.without(live.reference).entries().size() == 3,
                "forget leaves other sessions");
        check(SessionCatalog.figureDescription(live, live.reference, "")
                .contains("Live on local Core"), "live figure description");
        check(SessionCatalog.figureDescription(watched, watched.reference, "")
                .contains("Watching on studio"), "watched figure description");
        check(SessionCatalog.figureDescription(offline, offline.reference, "")
                .contains("Offline on loft"), "offline figure description");
        check(SessionCatalog.figureDescription(null,
                "hc://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/t1", "")
                .contains("Unavailable"), "unknown figure description");
        check(SessionCatalog.figureDescription(null, "", "duplicate terminal")
                .contains("Invalid binding"), "invalid figure description");
        check(SessionCatalog.empty().entries().isEmpty(), "empty catalogue");
        List<SessionCatalog.Entry> pinned = catalog.pinned();
        check(pinned.get(0) == live, "pinned identity");
        System.out.println("session catalogue tests passed (" + checks + " checks)");
    }
}
