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
}
