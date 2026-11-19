package com.emthin.pad;

import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;

/**
 * Bounded return history for one live document (C08).
 *
 * <p>Entries are destinations only; ordinary caret movement and scroll do not
 * call {@link #push}. The first entry is the location captured when the
 * document was installed, so Back has somewhere to return before the first
 * explicit jump. A new jump after Back truncates Forward.
 */
public final class NavigationController {
    public static final int MAX_ENTRIES = 30;

    /** A stable source/reading destination plus its local fold context. */
    public static final class Entry {
        public final String kind;
        public final String label;
        public final int start, end;
        public final String selectionAnchor;
        public final int selectionSide, selectionFallback;
        public final String readingAnchor;
        public final int readingSide, readingFallback;
        public final LinkedHashMap<String, String[]> folds;
        public final boolean inspector;
        /** Source inspector scope: whole document vs. the modifier at the caret. */
        public final boolean wholeDocument;

        public Entry(String kind, String label, int start, int end,
                     String selectionAnchor, int selectionSide, int selectionFallback,
                     String readingAnchor, int readingSide, int readingFallback,
                     LinkedHashMap<String, String[]> folds, boolean inspector,
                     boolean wholeDocument) {
            this.kind = kind == null ? "" : kind;
            this.label = label == null ? "" : label;
            this.start = Math.max(0, Math.min(start, end));
            this.end = Math.max(start, end);
            this.selectionAnchor = selectionAnchor == null ? "" : selectionAnchor;
            this.selectionSide = selectionSide;
            this.selectionFallback = selectionFallback;
            this.readingAnchor = readingAnchor == null ? "" : readingAnchor;
            this.readingSide = readingSide;
            this.readingFallback = readingFallback;
            this.folds = new LinkedHashMap<>();
            if (folds != null) {
                for (String key : folds.keySet()) {
                    String[] pair = folds.get(key);
                    if (pair != null) this.folds.put(key, new String[] {pair[0], pair[1]});
                }
            }
            this.inspector = inspector;
            this.wholeDocument = wholeDocument;
        }

        boolean sameDestination(Entry other) {
            return other != null && kind.equals(other.kind)
                    && start == other.start && end == other.end;
        }
    }

    private final ArrayList<Entry> entries = new ArrayList<>();
    private int index = -1;

    /** Clear history and install the current location as its baseline. */
    public void reset(Entry current) {
        entries.clear();
        index = -1;
        if (current != null) {
            entries.add(current);
            index = 0;
        }
    }

    public void clear() {
        entries.clear();
        index = -1;
    }

    public boolean canBack() {
        return index > 0;
    }

    public boolean canForward() {
        return index >= 0 && index + 1 < entries.size();
    }

    public Entry current() {
        return index >= 0 && index < entries.size() ? entries.get(index) : null;
    }

    /** Refresh the current entry to where the reader actually is now. */
    public boolean replaceCurrent(Entry current) {
        if (current == null) return false;
        if (index < 0) return false;
        entries.set(index, current);
        return true;
    }

    /**
     * Record an explicit accepted destination. Adjacent equal destinations
     * are ignored. Returns true when the history changed.
     */
    public boolean push(Entry destination) {
        if (destination == null) return false;
        if (entries.isEmpty()) {
            entries.add(destination);
            index = 0;
            return true;
        }
        if (destination.sameDestination(entries.get(index))) return false;
        while (entries.size() > index + 1) entries.remove(entries.size() - 1);
        entries.add(destination);
        index = entries.size() - 1;
        while (entries.size() > MAX_ENTRIES) {
            entries.remove(0);
            index--;
        }
        return true;
    }

    /** Move one destination back, returning it; null at the start. */
    public Entry back() {
        if (!canBack()) return null;
        index--;
        return entries.get(index);
    }

    /** Move one destination forward, returning it; null at the end. */
    public Entry forward() {
        if (!canForward()) return null;
        index++;
        return entries.get(index);
    }

    public int size() {
        return entries.size();
    }

    public int index() {
        return index;
    }

    public List<Entry> entries() {
        return Collections.unmodifiableList(new ArrayList<>(entries));
    }
}
