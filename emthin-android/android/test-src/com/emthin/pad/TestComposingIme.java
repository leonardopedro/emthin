package com.emthin.pad;

import android.inputmethodservice.InputMethodService;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;
import android.view.Gravity;
import android.view.View;
import android.view.inputmethod.InputConnection;
import android.widget.TextView;

/**
 * Test-only IME for the device lane. It has no user-facing keyboard logic;
 * the lane sends compose/finish/commit commands so the app's real
 * InputConnection composing path is exercised with CJK and emoji text.
 */
public class TestComposingIme extends InputMethodService {
    static final String TAG = "emthin-test-ime";
    static volatile TestComposingIme instance;
    private final Handler main = new Handler(Looper.getMainLooper());

    @Override public void onCreate() {
        super.onCreate();
        instance = this;
    }

    @Override public void onDestroy() {
        if (instance == this) instance = null;
        super.onDestroy();
    }

    @Override public View onCreateInputView() {
        TextView label = new TextView(this);
        label.setText("emthin test IME");
        label.setTextColor(0xFFE6EDF3);
        label.setBackgroundColor(0xFF1A2229);
        label.setGravity(Gravity.CENTER);
        label.setPadding(16, 16, 16, 16);
        return label;
    }

    static void dispatch(final String action, final String text) {
        final TestComposingIme ime = instance;
        if (ime == null) {
            Log.i(TAG, "missing-ime " + action);
            return;
        }
        ime.main.post(new Runnable() {
            @Override public void run() {
                InputConnection connection = ime.getCurrentInputConnection();
                if (connection == null) {
                    Log.i(TAG, "no-connection " + action);
                    return;
                }
                if ("compose".equals(action)) {
                    connection.setComposingText(text == null ? "" : text, 1);
                } else if ("finish".equals(action)) {
                    connection.finishComposingText();
                } else if ("commit".equals(action)) {
                    connection.commitText(text == null ? "" : text, 1);
                } else {
                    Log.i(TAG, "unknown-action " + action);
                    return;
                }
                Log.i(TAG, action + " " + (text == null ? "" : text));
            }
        });
    }
}
