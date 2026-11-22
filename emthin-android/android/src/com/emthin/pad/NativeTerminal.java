package com.emthin.pad;

import android.graphics.Bitmap;
import android.view.KeyEvent;

import org.json.JSONObject;

import java.nio.ByteBuffer;

/**
 * One page-local native terminal scene. The parser, retained rows and glyph
 * atlas live in Rust; this wrapper only owns the JNI handle and the reusable
 * destination bitmap.
 */
final class NativeTerminal {
    static {
        System.loadLibrary("emthin_pad");
    }

    private long handle;
    private int width;
    private int height;
    private int cols;
    private int rows;
    private Bitmap bitmap;

    NativeTerminal(int cols, int rows, int cellWidth, int cellHeight, int baseline, boolean echo) {
        this.cols = Math.max(1, Math.min(300, cols));
        this.rows = Math.max(1, Math.min(120, rows));
        handle = nativeCreate(this.cols, this.rows, cellWidth, cellHeight, baseline, echo);
    }

    boolean valid() {
        return handle != 0;
    }

    JSONObject feed(byte[] bytes) {
        return parse(nativeFeed(handle, bytes));
    }

    JSONObject state() {
        return parse(nativeState(handle));
    }

    JSONObject resize(int cols, int rows) {
        this.cols = Math.max(1, Math.min(300, cols));
        this.rows = Math.max(1, Math.min(120, rows));
        return parse(nativeResize(handle, this.cols, this.rows));
    }

    int cols() {
        return cols;
    }

    int rows() {
        return rows;
    }

    /** Whether the program enabled button or motion mouse reporting. */
    boolean mouseTracking() {
        JSONObject mouse = state().optJSONObject("mouse");
        return mouse != null && mouse.optBoolean("button", false);
    }

    boolean mouseMotionTracking() {
        JSONObject mouse = state().optJSONObject("mouse");
        return mouse != null && mouse.optBoolean("motion", false);
    }

    JSONObject mouseMotion(int x, int y, int button, int mods) {
        return parse(nativeMouseMotion(handle, x, y, button, mods));
    }

    /** Text in an inclusive cell rectangle, normalized by the native side. */
    String selection(int x0, int y0, int x1, int y1) {
        String text = nativeSelection(handle, x0, y0, x1, y1);
        return text == null ? "" : text;
    }

    /** Cell bounds for the word at a point as {x0,y0,x1,y1}, or null. */
    int[] wordSelection(int x, int y) {
        JSONObject bounds = parse(nativeWordSelection(handle, x, y));
        if (!bounds.has("x0")) return null;
        return new int[] {bounds.optInt("x0", 0), bounds.optInt("y0", 0),
                bounds.optInt("x1", 0), bounds.optInt("y1", 0)};
    }

    JSONObject key(int key, int mods, String text, int unshifted) {
        return parse(nativeKey(handle, key, mods, text == null ? "" : text, unshifted));
    }

    JSONObject text(String text) {
        return parse(nativeText(handle, text == null ? "" : text));
    }

    JSONObject mouse(int x, int y, int button, boolean press, int mods) {
        return parse(nativeMouse(handle, x, y, button, press, mods));
    }

    JSONObject recreate() {
        return parse(nativeRecreate(handle));
    }

    /** Reuses the destination bitmap when the scene grid has not changed. */
    synchronized Bitmap bitmap() {
        byte[] rgba = nativeRgba(handle);
        if (rgba == null || rgba.length < 4) return null;
        JSONObject state = state();
        int w = state.optInt("width", 0);
        int h = state.optInt("height", 0);
        if (w <= 0 || h <= 0 || w * h * 4 != rgba.length) return null;
        if (bitmap == null || bitmap.getWidth() != w || bitmap.getHeight() != h
                || bitmap.isRecycled()) {
            if (bitmap != null && !bitmap.isRecycled()) bitmap.recycle();
            bitmap = Bitmap.createBitmap(w, h, Bitmap.Config.ARGB_8888);
            bitmap.setDensity(Bitmap.DENSITY_NONE);
        }
        bitmap.copyPixelsFromBuffer(ByteBuffer.wrap(rgba));
        width = w;
        height = h;
        return bitmap;
    }

    int width() {
        return width;
    }

    int height() {
        return height;
    }

    void close() {
        if (bitmap != null && !bitmap.isRecycled()) bitmap.recycle();
        bitmap = null;
        if (handle != 0) {
            nativeClose(handle);
            handle = 0;
        }
    }

    private static native long nativeCreate(int cols, int rows, int cellWidth,
            int cellHeight, int baseline, boolean echo);
    private static native String nativeFeed(long handle, byte[] bytes);
    private static native String nativeState(long handle);
    private static native byte[] nativeRgba(long handle);
    private static native String nativeResize(long handle, int cols, int rows);
    private static native String nativeKey(long handle, int key, int mods, String text,
            int unshifted);
    private static native String nativeText(long handle, String text);
    private static native String nativeMouse(long handle, int x, int y, int button,
            boolean press, int mods);
    private static native String nativeMouseMotion(long handle, int x, int y, int button,
            int mods);
    private static native String nativeSelection(long handle, int x0, int y0, int x1, int y1);
    private static native String nativeWordSelection(long handle, int x, int y);
    private static native String nativeRecreate(long handle);
    private static native void nativeClose(long handle);

    private static JSONObject parse(String json) {
        try {
            return new JSONObject(json == null ? "{}" : json);
        } catch (Exception e) {
            return new JSONObject();
        }
    }

    /** Ghostty physical key code for an Android hardware key, or 0. */
    static int ghosttyKey(int androidKey) {
        switch (androidKey) {
            case KeyEvent.KEYCODE_ESCAPE: return 120;
            case KeyEvent.KEYCODE_ENTER: return 58;
            case KeyEvent.KEYCODE_DEL: return 53;
            case KeyEvent.KEYCODE_TAB: return 64;
            case KeyEvent.KEYCODE_SPACE: return 63;
            case KeyEvent.KEYCODE_DPAD_UP: return 78;
            case KeyEvent.KEYCODE_DPAD_DOWN: return 75;
            case KeyEvent.KEYCODE_DPAD_LEFT: return 76;
            case KeyEvent.KEYCODE_DPAD_RIGHT: return 77;
            case KeyEvent.KEYCODE_MOVE_HOME: return 71;
            case KeyEvent.KEYCODE_MOVE_END: return 69;
            case KeyEvent.KEYCODE_PAGE_UP: return 74;
            case KeyEvent.KEYCODE_PAGE_DOWN: return 73;
            case KeyEvent.KEYCODE_FORWARD_DEL: return 68;
            case KeyEvent.KEYCODE_INSERT: return 72;
            default:
                if (androidKey >= KeyEvent.KEYCODE_A && androidKey <= KeyEvent.KEYCODE_Z) {
                    return 20 + (androidKey - KeyEvent.KEYCODE_A);
                }
                if (androidKey >= KeyEvent.KEYCODE_0 && androidKey <= KeyEvent.KEYCODE_9) {
                    return 6 + (androidKey - KeyEvent.KEYCODE_0);
                }
                return 0;
        }
    }

    /** Ghostty modifier bits: shift 1, ctrl 2, alt 4, super 8. */
    static int ghosttyMods(KeyEvent event) {
        int mods = 0;
        if (event.isShiftPressed()) mods |= 1;
        if (event.isCtrlPressed()) mods |= 2;
        if (event.isAltPressed()) mods |= 4;
        if (event.isMetaPressed()) mods |= 8;
        return mods;
    }
}
