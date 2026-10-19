package com.emthin.pad;

import android.app.Activity;
import android.graphics.Bitmap;
import android.graphics.Color;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.text.Editable;
import android.text.Spannable;
import android.text.TextWatcher;
import android.text.style.BackgroundColorSpan;
import android.util.Base64;
import android.view.View;
import android.widget.Button;
import android.widget.EditText;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;

import org.json.JSONArray;
import org.json.JSONObject;

import java.nio.ByteBuffer;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

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

    /** Pages per render request — bounded so one JNI response stays small. */
    private static final int CHUNK = 3;
    /** Nominal A4 page width in points — Typst's default page size. */
    private static final float PAGE_WIDTH_PT = 595.28f;
    /** Vertical gap between page frames (dp). */
    private static final int PAGE_GAP_DP = 12;

    private long handle;
    private EditText editor;
    private Button undoButton;
    private Button redoButton;
    private TextView status;
    /** One caption + {@link ImageView} per rendered page, stacked. */
    private LinearLayout previewPages;
    private TextView previewError;
    /** True while we mutate the EditText ourselves (suppresses sync). */
    private boolean applying;

    /** Renders mathed off the UI thread (Typst layout is hundreds of ms). */
    private final ExecutorService renderer = Executors.newSingleThreadExecutor();
    private final Handler main = new Handler(Looper.getMainLooper());
    /** Bumped per edit; a worker result is dropped if a newer edit won. */
    private int renderGeneration;
    /** Pages currently stacked in the preview. */
    private int pagesLoaded;
    /** The document's full page count, from the latest chunk. */
    private int totalPages;
    /** One chunk in flight at a time (the renderer is single-threaded). */
    private boolean chunkInFlight;
    /** Debounce handle, so a burst of keystrokes rasterises once. */
    private final Runnable renderNow = new Runnable() {
        @Override
        public void run() {
            startRender(editor.getText().toString());
        }
    };

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
        main.removeCallbacks(renderNow);
        renderer.shutdown();
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

        // mathed preview: the document typeset by the same engine
        // velysterm's mathed_android blits (mathed_mini's paged
        // layout), so what the pad shows agrees with what a real mathed
        // frontend would produce. Page-by-page frames — mathed_android's
        // unit — so a long document streams in as the scroll reaches the
        // loaded end and scrolling never re-rasterises. The editor keeps
        // 1f of the height and the preview takes what is left, so
        // typing never hides it.
        TextView previewLabel = new TextView(this);
        previewLabel.setText("mathed render");
        previewLabel.setTextColor(Color.parseColor("#6E7A87"));
        previewLabel.setTextSize(11f);
        previewLabel.setPadding(0, dp(8), 0, dp(4));
        root.addView(previewLabel, new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT));

        previewPages = new LinearLayout(this);
        previewPages.setOrientation(LinearLayout.VERTICAL);

        previewError = new TextView(this);
        previewError.setTextColor(Color.parseColor("#E06C75"));
        previewError.setTextSize(11f);
        previewError.setVisibility(View.GONE);

        LinearLayout previewBox = new LinearLayout(this);
        previewBox.setOrientation(LinearLayout.VERTICAL);
        previewBox.addView(previewPages, new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT));
        previewBox.addView(previewError, new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT));

        ScrollView previewScroll = new ScrollView(this);
        previewScroll.setFillViewport(true);
        previewScroll.addView(previewBox);
        // Streaming pages in on scroll: when the scroll reaches the
        // loaded end (and the document has more), the next chunk is
        // requested. The whole point of paging is that this never
        // re-rasterises the pages already stacked below.
        previewScroll.setOnScrollChangeListener(new View.OnScrollChangeListener() {
            @Override
            public void onScrollChange(View v, int x, int y, int oldX, int oldY) {
                if (previewPages.getChildCount() == 0) {
                    return;
                }
                View last = previewPages.getChildAt(previewPages.getChildCount() - 1);
                if (y + v.getHeight() >= last.getBottom() - dp(PAGE_GAP_DP)) {
                    requestChunk(editor.getText().toString(), pagesLoaded);
                }
            }
        });
        root.addView(previewScroll, new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f));

        setContentView(root);
    }

    private int dp(int v) {
        return Math.round(v * getResources().getDisplayMetrics().density);
    }

    // ----- mathed rendering --------------------------------------------

    /** Debounces: a burst of keystrokes rasterises once, ~300 ms after the last. */
    private void scheduleRender() {
        main.removeCallbacks(renderNow);
        main.postDelayed(renderNow, 300);
    }

    /**
     * Starts a fresh page-frame render of {@code text}: the stacked
     * pages are dropped and chunk 0 streams in, unless a newer edit
     * supersedes the request while Typst is laying out.
     */
    private void startRender(final String text) {
        renderGeneration++;
        pagesLoaded = 0;
        totalPages = 0;
        chunkInFlight = false;
        previewPages.removeAllViews();
        requestChunk(text, 0);
    }

    /**
     * Renders pages {@code start..start + CHUNK} on the worker thread
     * and stacks them only if no newer edit superseded the request. At
     * most one chunk is in flight (the renderer is single-threaded), so
     * chunks always append in order.
     */
    private void requestChunk(final String text, final int start) {
        if (chunkInFlight || (totalPages > 0 && start >= totalPages)) {
            return; // one in flight already, or nothing left to load
        }
        final int generation = renderGeneration;
        chunkInFlight = true;
        // Fit the A4 page raster to the preview width at ≥ 1 px/pt so
        // text stays legible; the ImageView scales it the rest of the
        // way down. (Before the first layout the width is 0 → 1.0.)
        final float scale = Math.min(2.5f, Math.max(1.0f, previewPages.getWidth() / PAGE_WIDTH_PT));

        renderer.execute(new Runnable() {
            @Override
            public void run() {
                final String json = DocBridge.renderPages(text, start, CHUNK, scale);
                main.post(new Runnable() {
                    @Override
                    public void run() {
                        chunkInFlight = false;
                        if (generation == renderGeneration) {
                            applyChunk(json);
                        }
                    }
                });
            }
        });
    }

    /**
     * Shows a render diagnostic and stops the page stream until the next
     * edit (a half-typed expression must not retry per scroll tick).
     */
    private void showPreviewError(String message) {
        previewError.setText(message);
        previewError.setVisibility(View.VISIBLE);
        totalPages = pagesLoaded;
    }

    /** Decodes one chunk of {width,height,rgba} pages into page frames. */
    private void applyChunk(String json) {
        try {
            JSONObject o = new JSONObject(json);
            String error = o.optString("error", "");
            if (!error.isEmpty()) {
                showPreviewError("render: " + error);
                return;
            }

            totalPages = o.getInt("total");
            JSONArray pages = o.getJSONArray("pages");
            for (int i = 0; i < pages.length(); i++) {
                JSONObject p = pages.getJSONObject(i);
                int w = p.getInt("width");
                int h = p.getInt("height");
                byte[] rgba = Base64.decode(p.getString("rgba"), Base64.DEFAULT);
                if (rgba.length != w * h * 4 || w <= 0 || h <= 0) {
                    throw new IllegalStateException("bad raster " + w + "x" + h
                            + " (" + rgba.length + " bytes)");
                }

                Bitmap bmp = Bitmap.createBitmap(w, h, Bitmap.Config.ARGB_8888);
                // ARGB_8888's native buffer is RGBA byte order, which is
                // what the bridge encodes — a straight copy, no swizzle.
                bmp.copyPixelsFromBuffer(ByteBuffer.wrap(rgba));

                TextView caption = new TextView(this);
                caption.setText("page " + (pagesLoaded + 1) + " / " + totalPages);
                caption.setTextColor(Color.parseColor("#6E7A87"));
                caption.setTextSize(10f);
                caption.setPadding(0, dp(PAGE_GAP_DP), 0, dp(2));
                previewPages.addView(caption);

                ImageView pageView = new ImageView(this);
                pageView.setAdjustViewBounds(true);
                pageView.setBackgroundColor(Color.parseColor("#FAFBFC")); // mathed's paper white
                pageView.setImageBitmap(bmp);
                previewPages.addView(pageView);

                pagesLoaded++;
            }
            previewError.setVisibility(View.GONE);
        } catch (Exception e) {
            showPreviewError("render: " + e.getMessage());
        }
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
            scheduleRender();
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
