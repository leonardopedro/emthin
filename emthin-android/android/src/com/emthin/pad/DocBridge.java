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

    public static native void free(long handle);

    public static native String state(long handle);

    /** Pushes {@code text} into the model (one undo step) and moves the caret. */
    public static native String sync(long handle, String text, int caretUtf16);

    public static native String undo(long handle);

    public static native String redo(long handle);

    /**
     * Renders a mathed document to pixels, returning
     * {@code {width, height, rgba}} with {@code rgba} base64-encoded
     * RGBA8 — or {@code {error}} on a typesetting failure.
     *
     * <p><b>Stateless and safe to call off the UI thread.</b> It takes the
     * document <em>text</em>, never the handle, because Typst layout costs
     * hundreds of milliseconds and the handle is UI-thread-only. Copy the
     * text on the UI thread, rasterise on a worker, post the result back.
     */
    public static native String renderText(String docText, int widthPt);
}
