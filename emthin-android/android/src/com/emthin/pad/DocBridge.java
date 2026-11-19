package com.emthin.pad;

/**
 * Thin JNI bridge to the Rust core ({@code libemthin_pad.so}).
 *
 * <p>Every method takes the DocModel handle returned by {@link #create}
 * and returns the full editor state as a JSON string with the shape
 * {@code {text, revision, caret, canUndo, canRedo, segments:[{prop,
 * kind, span, stmt}]}}. All calls must happen on the UI thread.
 */
public final class DocBridge {
    static {
        System.loadLibrary("emthin_pad");
    }

    private DocBridge() {}

    public static native long create(String initialText);

    /** Returns zero for a corrupt snapshot or one that does not match the text. */
    public static native long createFromSnapshot(String snapshot, String expectedText);

    /** Open a Loro snapshot from a document file. Returns zero on invalid data. */
    public static native long openSnapshot(String snapshot);

    public static native String snapshot(long handle);

    /** Raw snapshot bytes captured on the UI thread for recovery/rendering. */
    public static native byte[] snapshotBytes(long handle);

    /** Open a raw Loro snapshot. Returns zero on invalid data. */
    public static native long openSnapshotBytes(byte[] snapshot);

    /**
     * SHA-256 of canonical rich-text content, including every attribute
     * name/value and excluding Loro history. Empty on an invalid snapshot.
     * Safe to call on a worker with captured bytes.
     */
    public static native String logicalFingerprint(byte[] snapshot);

    public static native void free(long handle);

    public static native String state(long handle);

    /**
     * Encodes a stable Loro cursor at a UTF-16 offset. The result contains
     * the base64 cursor, affinity and UTF-16 fallback; callers bind it to the
     * current document identity.
     */
    public static native String createAnchor(long handle, int offsetUtf16, int side);

    /** Resolves a stable cursor to a grapheme-snapped UTF-16 offset. */
    public static native int resolveAnchor(long handle, String encoded, int side,
                                           int fallbackUtf16);

    /** Mathed's incremental search, returned as UTF-16 ranges. */
    public static native String search(String text, String query);

    /**
     * One bounded, scoped search page. {@code scope} is text, source or
     * selection; {@code pageStart} -1 chooses the page containing the first
     * match at/after {@code anchorUtf16}. Returns total/replaceable/protected
     * counts and UTF-16 ranges, each marked when replacement must protect it.
     */
    public static native String searchScoped(String text, String query, String scope,
                                             int selectionStart, int selectionEnd,
                                             int anchorUtf16, int pageStart,
                                             int pageSize, int includeAll);

    /**
     * Applies a checked literal replacement plan as one undo step. The
     * document must still equal {@code expectedText}; ranges are revalidated
     * against marker/property ranges before any edit.
     */
    public static native String replaceMany(long handle, String expectedText, String opsJson);

    public static native int wordBoundary(String text, int caret, int direction);

    /**
     * Grapheme boundary in UTF-16 units using mathed's UAX #29 rules.
     * Direction -1 is the previous boundary, 0/1 the containing cluster's
     * start/end, and 2 the next boundary.
     */
    public static native int graphemeBoundary(String text, int offset, int direction);

    public static native String mathCompletion(String text, int caret);

    /**
     * Definitions, uses, sections and unresolved names from mathed's semantic
     * index, as UTF-16 source ranges. Stateless and safe on a worker.
     */
    public static native String semanticIndex(String text);

    /** Validates a rename against a freshly rebuilt index; returns name/uses. */
    public static native String renamePreview(String text, int defIndex, String newName);

    /** Applies that checked plan as one grouped undo step. */
    public static native String renameDefinition(long handle, String expectedText,
                                                int defIndex, String newName);

    /** Citation index with source snippets; resolving status loads the bound bibliography. */
    public static native String citationIndex(String text, boolean resolveStatus);
    /** Shared style/locale choices and their defaults. */
    public static native String citationChoices();
    /** One citation detail formatted with presentation-only style/locale overrides. */
    public static native String citationDetail(String text, int number, float width, float scale,
                                              String style, String locale);

    /** Pushes {@code text} into the model (one undo step) and moves the caret. */
    public static native String sync(long handle, String text, int caretUtf16);

    /** Returns mathed's generated marker for a typed #, or empty if escaped. */
    public static native String typedMarker(String text, int offsetUtf16);

    public static native String setModifier(long handle, String startId, String endId,
                                            int start, int end, int enabled);

    public static native String setMarkerTargets(long handle, String startId, String endId,
                                                 String markerPairs);

    /** Stateless comparison rendering; safe to call on a worker. */
    public static native String modifierComparisons(String text, String snapshot,
                                                    String startId, String endId,
                                                    float width, float scale);

    /** Incoming/outgoing relationship metadata without rendering any row. */
    public static native String modifierRelationships(String text, String snapshot,
                                                      String startId, String endId);

    /** One expanded relationship row, with and without its excluded modifier. */
    public static native String modifierComparisonRow(String text, String snapshot,
                                                      int targetStart, int targetEnd,
                                                      String excludeStart, String excludeEnd,
                                                      float width, float scale);

    public static native String undo(long handle);

    public static native String redo(long handle);

    /**
     * Renders a chunk of a mathed document's pages to pixels, returning
     * {@code {total, start, pages:[{width, height, rgba}, …]}} with each
     * {@code rgba} base64-encoded RGBA8 — or {@code {error}} on a
     * typesetting failure. {@code total} is the document's full page
     * count; a {@code start} past it yields an empty {@code pages}
     * ("nothing more to load"), not an error.
     *
     * <p><b>Stateless and safe to call off the UI thread.</b> It takes the
     * document <em>text</em>, never the handle, because Typst layout costs
     * hundreds of milliseconds and the handle is UI-thread-only. Copy the
     * text on the UI thread, rasterise on a worker, post the result back.
     *
     * <p><b>Paged on purpose:</b> the layout is Typst's own page model (the
     * PDF-viewer unit), so a long document streams in chunks and scrolling
     * a preview never re-rasterises. {@code scale} is pixels per point
     * (non-positive falls back to 1.0); {@code maxPages} is clamped to
     * 1..8 per call.
     *
     * <p>{@code caretUtf16} is the editor caret as a UTF-16 index. It drives
     * reveal-on-caret exactly like the engine: the raw token under the caret
     * and the caret's collapsible expand, while a caret on plain prose keeps
     * the collapsed view (cite labels shown, tokens hidden).
     */
    public static native String renderPages(String docText, int start, int maxPages,
                                            float scale, int caretUtf16);

    public static native String renderPagesWithSnapshot(String docText, String snapshot,
            int start, int maxPages, float scale, int caretUtf16, int showMarkers, String expanded);

    /**
     * Compile (or reuse) the one retained worker layout for captured snapshot
     * bytes. Returns page count, caption position and modifier warnings.
     * Safe on the render worker; no live model handle is passed.
     */
    public static native String prepareLayout(byte[] snapshot, String docText,
            String layoutId, int showMarkers, String expanded, float textSizePt);

    /**
     * Rasterize one page from a prepared layout and retain glyph metadata only
     * for the inclusive page window. Safe on the render worker.
     */
    public static native String renderLayoutPage(String layoutId, String docText,
            int pageIndex, float scale, long pixelLimit,
            int retainFrom, int retainTo);

    /** Locate a UTF-16 range in a prepared layout without rasterizing pages. */
    public static native String locateLayoutRange(String layoutId, String docText,
            int startUtf16, int endUtf16);

    /**
     * Compiles a CPS constant-return program with the australVM
     * Cranelift JIT and executes it in-process, returning
     * {@code {"ok": <result>, "entry": "run_<n>"}} — or
     * {@code {"error": "…"}} if the compile or run fails (a failed JIT
     * is data, not a crash, like {@link #renderPages}).
     *
     * <p><b>Safe off the UI thread</b>: the JIT engine is per-thread, so
     * two concurrent calls each get their own engine. The auth gate
     * stays at its secure DenyAll default — these programs make no
     * calls, so nothing here relaxes policy to make the smoke pass.
     */
    public static native String jitRoundtrip(long value);
}
