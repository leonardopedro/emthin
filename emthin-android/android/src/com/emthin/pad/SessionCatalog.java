package com.emthin.pad;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

/** The Sessions picker model: public terminal references only.
 *
 * <p>No socket path, endpoint, credential or host registry key belongs here.
 * The fake catalogue lets the pad foundation be tested before a Core transport
 * exists.
 */
public final class SessionCatalog {
    public enum State { LIVE, FINISHED, WATCHED, OFFLINE, UNAVAILABLE }

    public static final class Entry {
        public final String reference;
        public final String caption;
        public final String hostLabel;
        public final State state;
        public final boolean pinned;

        Entry(String reference, String caption, String hostLabel, State state,
                boolean pinned) {
            this.reference = reference;
            this.caption = caption;
            this.hostLabel = hostLabel;
            this.state = state;
            this.pinned = pinned;
        }

        public String stateLabel() {
            switch (state) {
                case LIVE: return "Live";
                case FINISHED: return "Finished";
                case WATCHED: return "Watching";
                case OFFLINE: return "Offline";
                case UNAVAILABLE: return "Unavailable";
                default: throw new IllegalStateException("unhandled " + state);
            }
        }

        public boolean canPin() {
            return state == State.LIVE || state == State.WATCHED;
        }

        public boolean canDetach() {
            return state == State.WATCHED;
        }
    }

    private final List<Entry> entries;

    private SessionCatalog(List<Entry> entries) {
        this.entries = Collections.unmodifiableList(new ArrayList<>(entries));
    }

    public static SessionCatalog fake() {
        List<Entry> entries = new ArrayList<>();
        entries.add(new Entry(
                "hc://6ab09a20fd1288983dfbbac2bb349231/t3",
                "Fleet · Infinity", "local Core", State.LIVE, true));
        entries.add(new Entry(
                "hc://0123456789abcdef0123456789abcdef/t7",
                "Build log", "studio", State.WATCHED, false));
        entries.add(new Entry(
                "hc://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/t9",
                "Remote shell", "loft", State.OFFLINE, false));
        entries.add(new Entry(
                "herdr-local://fedcba9876543210fedcba9876543210/pane%20one",
                "Legacy pane", "Herdr 0.9.5", State.UNAVAILABLE, false));
        return new SessionCatalog(entries);
    }

    public static SessionCatalog empty() {
        return new SessionCatalog(Collections.emptyList());
    }

    public List<Entry> entries() {
        return entries;
    }

    public List<Entry> pinned() {
        List<Entry> pinned = new ArrayList<>();
        for (Entry entry : entries) {
            if (entry.pinned) pinned.add(entry);
        }
        return pinned;
    }

    public Entry byReference(String reference) {
        for (Entry entry : entries) {
            if (entry.reference.equals(reference)) return entry;
        }
        return null;
    }

    /** Mark a document-pinned session in the live catalogue. */
    public SessionCatalog pin(String reference) {
        return replace(reference, true, null);
    }

    /**
     * Release this client's watcher without removing the session from the
     * host. The live entry stays visible and can be watched or pinned again.
     */
    public SessionCatalog detach(String reference) {
        Entry entry = byReference(reference);
        if (entry == null || !entry.canDetach()) return this;
        return replace(reference, entry.pinned, State.LIVE);
    }

    /** Fake-host removal used by tests and explicit forget flows. */
    public SessionCatalog without(String reference) {
        List<Entry> remaining = new ArrayList<>();
        for (Entry entry : entries) {
            if (!entry.reference.equals(reference)) remaining.add(entry);
        }
        return new SessionCatalog(remaining);
    }

    private SessionCatalog replace(String reference, boolean pinned, State state) {
        List<Entry> next = new ArrayList<>();
        for (Entry entry : entries) {
            if (entry.reference.equals(reference)) {
                next.add(new Entry(entry.reference, entry.caption, entry.hostLabel,
                        state == null ? entry.state : state, pinned));
            } else {
                next.add(entry);
            }
        }
        return new SessionCatalog(next);
    }

    /** TalkBack description for one document figure and its live state. */
    public static String figureDescription(Entry entry, String reference, String bindingError) {
        String caption = entry == null ? "Terminal figure" : entry.caption;
        if (bindingError != null && !bindingError.isEmpty()) {
            return caption + ". Invalid binding. " + bindingError + ". Activate for Sessions.";
        }
        if (reference == null || reference.isEmpty()) return caption + ". Unbound figure.";
        if (entry == null) return caption + ". Unavailable. Activate for Sessions.";
        String host = entry.hostLabel == null || entry.hostLabel.isEmpty()
                ? "" : " on " + entry.hostLabel;
        return caption + ". " + entry.stateLabel() + host
                + ". Activate to go to figure.";
    }

    public static String actionLabel(String action, Entry entry) {
        if ("PIN".equals(action)) return entry.canPin() ? "Pin to document" : "Unavailable";
        if ("REBIND".equals(action)) return "Rebind";
        if ("GO_TO_FIGURE".equals(action)) return "Go to figure";
        if ("DETACH".equals(action)) return entry.canDetach() ? "Detach" : "Not attached";
        throw new IllegalArgumentException("unknown action " + action);
    }
}
