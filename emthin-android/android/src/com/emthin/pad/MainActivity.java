package com.emthin.pad;

import android.app.Activity;
import android.graphics.Color;
import android.os.Bundle;
import android.text.Editable;
import android.text.Spannable;
import android.text.TextWatcher;
import android.text.style.BackgroundColorSpan;
import android.view.View;
import android.widget.Button;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.TextView;

import org.json.JSONArray;
import org.json.JSONObject;

/**
 * emthin pad — a plain {@link EditText} driven by the Rust-side emthin
 * {@code DocModel} (compiled verbatim from emthin's docui/model.rs).
 *
 * <p>Flow: every text change is pushed into the model with
 * {@link DocBridge#sync}, which applies the minimal diff as one undo
 * step and returns the full editor state as JSON. The returned state
 * re-syncs undo/redo buttons, the revision/segment status line, and the
 * resolved-marker highlights. Programmatic changes (undo/redo, model
 * divergence) set the caret from the state; typing never moves the caret
 * behind the user's back.
 */
public class MainActivity extends Activity {

    private static final String STARTER =
            "#1 terminal #2 \\app(#1, #2, 600, 300, \"term\")\n"
                    + "#3 editor #4 \\app(#3, #4, 400, 300, \"vim\")\n";

    /** Translucent highlight over ranges the model resolved to segments. */
    private static final int SEGMENT_HIGHLIGHT = 0x3346B883;

    private long handle;
    private EditText editor;
    private Button undoButton;
    private Button redoButton;
    private TextView status;
    /** True while we mutate the EditText ourselves (suppresses sync). */
    private boolean applying;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);

        String initial = STARTER;
        if (savedInstanceState != null) {
            String saved = savedInstanceState.getString("docText");
            if (saved != null) {
                initial = saved;
            }
        }

        buildUi();
        handle = DocBridge.create(initial);
        applyState(DocBridge.state(handle), true);

        editor.addTextChangedListener(new TextWatcher() {
            @Override
            public void beforeTextChanged(CharSequence s, int st, int c, int a) {}

            @Override
            public void onTextChanged(CharSequence s, int st, int b, int c) {}

            @Override
            public void afterTextChanged(Editable s) {
                if (applying || handle == 0) {
                    return;
                }
                int caret = editor.getSelectionStart();
                String state = DocBridge.sync(handle, s.toString(), caret);
                applyState(state, false);
            }
        });
    }

    @Override
    protected void onSaveInstanceState(Bundle outState) {
        super.onSaveInstanceState(outState);
        if (editor != null) {
            outState.putString("docText", editor.getText().toString());
        }
    }

    @Override
    protected void onDestroy() {
        if (handle != 0) {
            DocBridge.free(handle);
            handle = 0;
        }
        super.onDestroy();
    }

    // ----- UI ----------------------------------------------------------

    private void buildUi() {
        LinearLayout root = new LinearLayout(this);
        root.setOrientation(LinearLayout.VERTICAL);
        root.setBackgroundColor(Color.parseColor("#101418"));
        root.setPadding(dp(12), dp(12), dp(12), dp(12));

        LinearLayout bar = new LinearLayout(this);
        bar.setOrientation(LinearLayout.HORIZONTAL);

        undoButton = new Button(this);
        undoButton.setText("Undo");
        undoButton.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                applyState(DocBridge.undo(handle), true);
            }
        });

        redoButton = new Button(this);
        redoButton.setText("Redo");
        redoButton.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                applyState(DocBridge.redo(handle), true);
            }
        });

        status = new TextView(this);
        status.setTextColor(Color.parseColor("#9BA7B4"));
        status.setTextSize(13f);
        status.setGravity(android.view.Gravity.END);
        LinearLayout.LayoutParams statusLp =
                new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f);
        statusLp.leftMargin = dp(12);
        status.setPadding(0, dp(14), 0, 0);

        bar.addView(undoButton);
        bar.addView(redoButton);
        bar.addView(status, statusLp);
        root.addView(bar, new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT));

        editor = new EditText(this);
        editor.setTypeface(android.graphics.Typeface.MONOSPACE);
        editor.setTextColor(Color.parseColor("#E6EDF3"));
        editor.setHintTextColor(Color.parseColor("#4E5965"));
        editor.setHint("#1 caption #2 \\app(#1, #2, w, h)");
        editor.setBackground(null);
        editor.setPadding(0, dp(12), 0, 0);
        editor.setGravity(android.view.Gravity.TOP | android.view.Gravity.START);
        root.addView(editor, new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f));

        setContentView(root);
    }

    private int dp(int v) {
        return Math.round(v * getResources().getDisplayMetrics().density);
    }

    // ----- state -------------------------------------------------------

    /**
     * Applies a JSON state string from the bridge.
     *
     * @param setCaret force the caret from the state (undo/redo); on
     *                 plain typing the editor already owns the caret and
     *                 the state caret just echoes it back.
     */
    private void applyState(String json, boolean setCaret) {
        try {
            JSONObject o = new JSONObject(json);

            String text = o.getString("text");
            boolean diverged = !text.contentEquals(editor.getText());
            if (diverged) {
                applying = true;
                editor.setText(text);
                applying = false;
            }
            if (diverged || setCaret) {
                int caret = Math.max(0, Math.min(o.getInt("caret"), editor.length()));
                editor.setSelection(caret);
            }

            undoButton.setEnabled(o.getBoolean("canUndo"));
            redoButton.setEnabled(o.getBoolean("canRedo"));
            refreshSegments(o);
        } catch (Exception e) {
            status.setText("state error: " + e.getMessage());
        }
    }

    /** Re-highlights resolved segments and refreshes the status line. */
    private void refreshSegments(JSONObject o) throws Exception {
        Editable ed = editor.getText();
        for (BackgroundColorSpan s : ed.getSpans(0, ed.length(), BackgroundColorSpan.class)) {
            ed.removeSpan(s);
        }

        JSONArray segs = o.getJSONArray("segments");
        StringBuilder props = new StringBuilder();
        int shown = 0;
        for (int i = 0; i < segs.length(); i++) {
            JSONObject s = segs.getJSONObject(i);
            JSONArray span = s.optJSONArray("span");
            if (span != null && span.length() == 2) {
                int start = byteToUtf16(ed.toString(), span.getInt(0));
                int end = byteToUtf16(ed.toString(), span.getInt(1));
                if (start < end && end <= ed.length()) {
                    ed.setSpan(new BackgroundColorSpan(SEGMENT_HIGHLIGHT), start, end,
                            Spannable.SPAN_EXCLUSIVE_EXCLUSIVE);
                }
            }
            if (shown < 3) {
                if (shown > 0) {
                    props.append(", ");
                }
                props.append(s.getString("prop"));
                shown++;
            }
        }

        String extra = segs.length() > shown ? ", +more" : "";
        status.setText("rev " + o.getLong("revision") + " · " + segs.length()
                + " segs · " + props + extra);
    }

    /**
     * UTF-8 byte offset → UTF-16 index (model spans are byte offsets;
     * Java strings are UTF-16). Snaps to the containing code point's
     * start; out-of-range offsets clamp to the string ends.
     */
    private static int byteToUtf16(String s, int byteIdx) {
        if (byteIdx <= 0) {
            return 0;
        }
        int bytes = 0;
        int i = 0;
        while (i < s.length()) {
            int cp = s.codePointAt(i);
            int n = utf8Len(cp);
            if (bytes + n > byteIdx) {
                return i;
            }
            bytes += n;
            i += Character.charCount(cp);
            if (bytes == byteIdx) {
                return i;
            }
        }
        return s.length();
    }

    private static int utf8Len(int cp) {
        if (cp < 0x80) {
            return 1;
        }
        if (cp < 0x800) {
            return 2;
        }
        if (cp < 0x10000) {
            return 3;
        }
        return 4;
    }
}
