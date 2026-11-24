package com.emthin.pad;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Set;

/** The Sessions picker model: public terminal references only.
 *
 * <p>No socket path, endpoint, credential or host registry key belongs here.
 * Entries come from each paired host's `sessions.list` (see
 * {@link GatewayCatalog}); {@link #fixture()} is the device lane's scripted
 * catalogue for the pin step, which runs without a gateway.
 */
public final class SessionCatalog {
    public enum State { LIVE, FINISHED, WATCHED, OFFLINE, STARTING, UNAVAILABLE }

    public static final class Entry {
        public final String reference;
        public final String caption;
        /** Registry key of the host that answered, so one host refreshes alone. */
        public final String host;
        public final String hostLabel;
        public final State state;
        public final boolean pinned;

        Entry(String reference, String caption, String host, String hostLabel, State state,
                boolean pinned) {
            this.reference = reference;
            this.caption = caption;
            this.host = host;
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
                case STARTING: return "Starting";
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

    /**
     * One paired host's last catalogue answer. {@code error} is null when the
     * host answered and a short reason when it did not, so the picker can show
     * a host row with Reconnect instead of an error dialog.
     */
    public static final class Host {
        public final String host;
        public final String label;
        public final String error;

        public Host(String host, String label, String error) {
            this.host = host;
            this.label = label == null || label.isEmpty() ? host : label;
            this.error = error;
        }

        public boolean reachable() {
            return error == null;
        }
    }

    private final List<Host> hosts;
    private final List<Entry> entries;

    private SessionCatalog(List<Host> hosts, List<Entry> entries) {
        this.hosts = Collections.unmodifiableList(new ArrayList<>(hosts));
        this.entries = Collections.unmodifiableList(new ArrayList<>(entries));
    }

    /**
     * The merged catalogue of every paired host: their rows in endpoint order,
     * with {@code pinned} naming the references the open document holds.
     */
    public static SessionCatalog of(List<Host> hosts, List<Entry> entries, Set<String> pinned) {
        return new SessionCatalog(hosts, markPinned(entries, pinned));
    }

    /**
     * One host's answer replacing its rows, every other host kept as it was.
     *
     * <p>Reconnect is per host: a picker with three paired hosts must not make
     * the two that answered wait for the one that did not. Rows of the
     * refreshed host keep their place in endpoint order, and the whole list is
     * re-marked against {@code pinned} because the document may have changed
     * since the last fetch.
     */
    public SessionCatalog withHost(Host updated, List<Entry> rows, Set<String> pinned) {
        List<Host> nextHosts = new ArrayList<>(hosts.size() + 1);
        boolean replaced = false;
        for (Host host : hosts) {
            if (host.host.equals(updated.host)) {
                nextHosts.add(updated);
                replaced = true;
            } else {
                nextHosts.add(host);
            }
        }
        if (!replaced) nextHosts.add(updated);
        List<Entry> nextEntries = new ArrayList<>(entries.size());
        for (Host host : nextHosts) {
            if (host.host.equals(updated.host)) {
                if (rows != null) nextEntries.addAll(rows);
                continue;
            }
            for (Entry entry : entries) {
                if (entry.host.equals(host.host)) nextEntries.add(entry);
            }
        }
        return new SessionCatalog(nextHosts, markPinned(nextEntries, pinned));
    }

    private static List<Entry> markPinned(List<Entry> entries, Set<String> pinned) {
        List<Entry> marked = new ArrayList<>(entries.size());
        for (Entry entry : entries) {
            boolean held = pinned != null && pinned.contains(entry.reference);
            marked.add(held == entry.pinned ? entry
                    : new Entry(entry.reference, entry.caption, entry.host,
                            entry.hostLabel, entry.state, held));
        }
        return marked;
    }

    /** The device lane's scripted catalogue for the pin step. Not live data. */
    public static SessionCatalog fixture() {
        List<Entry> entries = new ArrayList<>();
        entries.add(new Entry(
                "hc://6ab09a20fd1288983dfbbac2bb349231/t3",
                "Fleet · Infinity", "fixture-core", "local Core", State.LIVE, true));
        entries.add(new Entry(
                "hc://0123456789abcdef0123456789abcdef/t7",
                "Build log", "studio", "studio", State.WATCHED, false));
        entries.add(new Entry(
                "hc://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/t9",
                "Remote shell", "loft", "loft", State.OFFLINE, false));
        entries.add(new Entry(
                "herdr-local://fedcba9876543210fedcba9876543210/pane%20one",
                "Legacy pane", "loft", "Herdr 0.9.5", State.UNAVAILABLE, false));
        List<Host> hosts = new ArrayList<>();
        hosts.add(new Host("fixture-core", "local Core", null));
        hosts.add(new Host("studio", "studio", null));
        hosts.add(new Host("loft", "loft", null));
        return new SessionCatalog(hosts, entries);
    }

    public static SessionCatalog empty() {
        return new SessionCatalog(Collections.<Host>emptyList(), Collections.<Entry>emptyList());
    }

    public List<Entry> entries() {
        return entries;
    }

    /** Every paired host, in registry order, with its last fetch outcome. */
    public List<Host> hosts() {
        return hosts;
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
        return new SessionCatalog(hosts, remaining);
    }

    private SessionCatalog replace(String reference, boolean pinned, State state) {
        List<Entry> next = new ArrayList<>();
        for (Entry entry : entries) {
            if (entry.reference.equals(reference)) {
                next.add(new Entry(entry.reference, entry.caption, entry.host,
                        entry.hostLabel, state == null ? entry.state : state, pinned));
            } else {
                next.add(entry);
            }
        }
        return new SessionCatalog(hosts, next);
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
