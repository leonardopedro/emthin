package com.emthin.pad;

import java.util.concurrent.Executor;

/**
 * Find state independent of Android views. Search work runs on an injected
 * executor; results are accepted only for the newest text/query/scope request.
 * Only one 200-result page is retained, so a long document does not leave an
 * unbounded JSON range array on the UI thread.
 */
public final class FindController {
    public static final int SCOPE_TEXT = 0;
    public static final int SCOPE_SOURCE = 1;
    public static final int SCOPE_SELECTION = 2;
    public static final int PAGE_SIZE = 200;

    public static final class Match {
        public final int start;
        public final int end;
        public final boolean protectedRange;

        public Match(int start, int end, boolean protectedRange) {
            this.start = start;
            this.end = end;
            this.protectedRange = protectedRange;
        }
    }

    public static final class Page {
        public final int total;
        public final int replaceable;
        public final int protectedCount;
        public final int first;
        public final int pageStart;
        public final Match[] matches;
        public final String error;

        public Page(int total, int replaceable, int protectedCount, int first,
                    int pageStart, Match[] matches, String error) {
            this.total = Math.max(0, total);
            this.replaceable = Math.max(0, replaceable);
            this.protectedCount = Math.max(0, protectedCount);
            this.first = Math.max(0, first);
            this.pageStart = Math.max(0, pageStart);
            this.matches = matches == null ? new Match[0] : matches;
            this.error = error;
        }

        public static Page empty() { return new Page(0, 0, 0, 0, 0, new Match[0], null); }
    }

    public static final class Request {
        public final String text;
        public final String query;
        public final int scope;
        public final int selectionStart;
        public final int selectionEnd;
        public final int anchor;
        public final int pageStart;
        public final int pageSize;
        public final boolean includeAll;

        Request(String text, String query, int scope, int selectionStart, int selectionEnd,
                int anchor, int pageStart, int pageSize, boolean includeAll) {
            this.text = text;
            this.query = query;
            this.scope = scope;
            this.selectionStart = selectionStart;
            this.selectionEnd = selectionEnd;
            this.anchor = anchor;
            this.pageStart = pageStart;
            this.pageSize = pageSize;
            this.includeAll = includeAll;
        }
    }

    /** Blocking search adapter; called on {@code searchWorker}, never the UI. */
    public interface SearchFn {
        Page search(Request request) throws Exception;
    }

    public interface Listener {
        void onFindChanged();
    }

    public interface PlanListener {
        void onPlan(Page page, String error);
    }

    private final SearchFn searchFn;
    private final Executor searchWorker;
    private final Executor callback;
    private final Listener listener;

    private String originText = "";
    private int originCaret;
    private int frozenSelectionStart;
    private int frozenSelectionEnd;
    private int scope = SCOPE_TEXT;
    private String query = "";
    private Page page = Page.empty();
    private int localCurrent = -1;
    private int pendingGlobal = -1;
    private boolean loading;
    private boolean wrapped;
    private String error;
    private long requestToken;

    public FindController(SearchFn searchFn, Executor searchWorker, Executor callback,
                          Listener listener) {
        this.searchFn = searchFn;
        this.searchWorker = searchWorker;
        this.callback = callback;
        this.listener = listener;
    }

    public void open(String text, int caret, int selectionStart, int selectionEnd) {
        originText = text == null ? "" : text;
        originCaret = Math.max(0, Math.min(caret, originText.length()));
        frozenSelectionStart = Math.max(0, Math.min(selectionStart, originText.length()));
        frozenSelectionEnd = Math.max(0, Math.min(selectionEnd, originText.length()));
        if (frozenSelectionStart > frozenSelectionEnd) {
            int swap = frozenSelectionStart;
            frozenSelectionStart = frozenSelectionEnd;
            frozenSelectionEnd = swap;
        }
        scope = SCOPE_TEXT;
        query = "";
        page = Page.empty();
        localCurrent = -1;
        pendingGlobal = -1;
        loading = false;
        wrapped = false;
        error = null;
        requestToken++;
    }

    public int originCaret() { return originCaret; }
    public String originText() { return originText; }
    public int scope() { return scope; }
    public String query() { return query; }
    public boolean isLoading() { return loading; }
    public boolean selectionAvailable() { return frozenSelectionStart < frozenSelectionEnd; }
    public int total() { return page.total; }
    public int replaceable() { return page.replaceable; }
    public int protectedCount() { return page.protectedCount; }
    public int pageStart() { return page.pageStart; }
    public Match[] pageMatches() { return page.matches; }

    public boolean setScope(String text, int nextScope) {
        if (nextScope == SCOPE_SELECTION && !selectionAvailable()) return false;
        if (scope == nextScope) return true;
        scope = nextScope;
        queryChanged(text, query);
        return true;
    }

    public void queryChanged(String text, String nextQuery) {
        query = nextQuery == null ? "" : nextQuery;
        page = Page.empty();
        localCurrent = -1;
        pendingGlobal = -1;
        wrapped = false;
        error = null;
        if (query.isEmpty()) {
            loading = false;
            requestToken++;
            notifyChanged();
            return;
        }
        request(text, -1, -1);
    }

    /** Starts a fresh current match at/after an explicit source offset. */
    public void searchFrom(String text, int anchor) {
        if (query.isEmpty()) return;
        pendingGlobal = -1;
        wrapped = false;
        request(text, -1, Math.max(0, anchor));
    }

    /** Recomputes after an edit, keeping the current match's position when possible. */
    public void documentChanged(String text) {
        int anchor = currentMatch() == null ? originCaret : currentMatch().start;
        pendingGlobal = -1;
        if (!query.isEmpty()) request(text, -1, anchor);
    }

    public Match currentMatch() {
        int index = localCurrent;
        if (index < 0 || index >= page.matches.length) return null;
        return page.matches[index];
    }

    public int currentGlobal() {
        if (localCurrent < 0 || localCurrent >= page.matches.length) return -1;
        return page.pageStart + localCurrent;
    }

    public int currentNumber() {
        int global = currentGlobal();
        return global < 0 ? 0 : global + 1;
    }

    public String status() {
        if (error != null) return error;
        if (loading) return query.isEmpty() ? "" : "searching\u2026";
        if (query.isEmpty()) return "";
        if (page.total == 0) return "No matches";
        return (wrapped ? "Wrapped \u00b7 " : "")
                + currentNumber() + " / " + page.total;
    }

    public void next(String text) {
        move(text, 1);
    }

    public void previous(String text) {
        move(text, -1);
    }

    private void move(String text, int direction) {
        if (page.total == 0) return;
        int global = currentGlobal();
        if (global < 0) global = page.first;
        int target = global + direction;
        if (target >= page.total) {
            target = 0;
            wrapped = true;
        } else if (target < 0) {
            target = page.total - 1;
            wrapped = true;
        } else {
            wrapped = false;
        }
        if (target >= page.pageStart && target < page.pageStart + page.matches.length) {
            localCurrent = target - page.pageStart;
            pendingGlobal = -1;
            notifyChanged();
            return;
        }
        pendingGlobal = target;
        request(text, (target / PAGE_SIZE) * PAGE_SIZE, currentMatch() == null
                ? originCaret : currentMatch().start);
    }

    public boolean selectGlobal(int global, String text) {
        if (global < 0 || global >= page.total) return false;
        if (global >= page.pageStart && global < page.pageStart + page.matches.length) {
            localCurrent = global - page.pageStart;
            pendingGlobal = -1;
            wrapped = false;
            notifyChanged();
            return true;
        }
        pendingGlobal = global;
        request(text, (global / PAGE_SIZE) * PAGE_SIZE, originCaret);
        return false;
    }

    public void accept(String text) {
        wrapped = false;
        error = null;
        notifyChanged();
    }

    public void cancel() {
        requestToken++;
        loading = false;
        pendingGlobal = -1;
        page = Page.empty();
        localCurrent = -1;
        query = "";
        wrapped = false;
        error = null;
        notifyChanged();
    }

    /** Requests the complete eligible plan for Replace all (capped natively). */
    public void requestPlan(String text, final PlanListener planListener) {
        final long token = ++requestToken;
        final Request request = new Request(text == null ? "" : text, query, scope,
                frozenSelectionStart, frozenSelectionEnd, originCaret, 0, 10_000, true);
        searchWorker.execute(new Runnable() {
            @Override public void run() {
                Page result;
                String failure = null;
                try {
                    result = searchFn.search(request);
                } catch (Exception exception) {
                    result = Page.empty();
                    failure = "Search unavailable";
                }
                final Page value = result;
                final String errorValue = failure;
                callback.execute(new Runnable() {
                    @Override public void run() {
                        if (token != requestToken) return;
                        planListener.onPlan(value, errorValue);
                    }
                });
            }
        });
    }

    private void request(String text, int pageStart, int anchor) {
        final long token = ++requestToken;
        loading = true;
        error = null;
        notifyChanged();
        final Request request = new Request(text == null ? "" : text, query, scope,
                frozenSelectionStart, frozenSelectionEnd, anchor, pageStart, PAGE_SIZE, false);
        searchWorker.execute(new Runnable() {
            @Override public void run() {
                Page result;
                String failure = null;
                try {
                    result = searchFn.search(request);
                    if (result == null) result = Page.empty();
                    if (result.error != null) failure = result.error;
                } catch (Exception exception) {
                    result = Page.empty();
                    failure = "Search unavailable";
                }
                final Page value = result;
                final String errorValue = failure;
                callback.execute(new Runnable() {
                    @Override public void run() {
                        if (token != requestToken) return;
                        acceptResult(value, errorValue);
                    }
                });
            }
        });
    }

    private void acceptResult(Page value, String failure) {
        loading = false;
        error = failure;
        page = value;
        if (page.matches.length == 0 || page.total == 0) {
            localCurrent = -1;
            pendingGlobal = -1;
            notifyChanged();
            return;
        }
        int target = pendingGlobal;
        if (target < 0) target = page.first;
        localCurrent = target - page.pageStart;
        if (localCurrent < 0) localCurrent = 0;
        if (localCurrent >= page.matches.length) localCurrent = page.matches.length - 1;
        pendingGlobal = -1;
        notifyChanged();
    }

    private void notifyChanged() {
        if (listener != null) listener.onFindChanged();
    }
}
