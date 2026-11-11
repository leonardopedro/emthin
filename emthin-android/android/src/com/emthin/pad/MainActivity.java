package com.emthin.pad;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.DialogInterface;
import android.content.SharedPreferences;
import android.content.Intent;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.graphics.Color;
import android.graphics.Canvas;
import android.graphics.Paint;
import android.graphics.drawable.BitmapDrawable;
import android.graphics.drawable.Drawable;
import android.graphics.drawable.GradientDrawable;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.net.Uri;
import android.text.Editable;
import android.text.InputType;
import android.text.Spannable;
import android.text.TextWatcher;
import android.text.style.BackgroundColorSpan;
import android.text.style.ReplacementSpan;
import android.util.Base64;
import android.util.Log;
import android.view.MotionEvent;
import android.view.KeyEvent;
import android.view.View;
import android.view.ViewTreeObserver;
import android.view.inputmethod.EditorInfo;
import android.widget.HorizontalScrollView;
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

    /** Where the pad keeps the document between process launches. */
    private static final String PREFS = "emthin-pad";
    private static final String PREF_DOC = "docText";
    private static final String PREF_SNAPSHOT = "docSnapshot";
    private static final String PREF_CARET = "caret";
    private static final String PREF_SCROLL = "scrollY";
    private static final String PREF_SCROLL_WIDTH = "scrollWidth";
    private static final String PREF_SHOW_MARKERS = "showMarkers";

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
    private TextView undoButton;
    private TextView redoButton;
    private TextView markersButton;
    private EditText targetsField;
    private String appliedTargetList = "";
    private boolean showMarkers;
    private TextView status;
    private LinearLayout searchBar;
    private EditText searchField;
    private TextView searchCount;
    private JSONArray searchMatches = new JSONArray();
    private int searchCurrent = -1;
    private int searchOrigin;
    private int selectionAnchor = -1;
    private boolean pasting;
    private static final int OPEN_DOCUMENT = 41;
    private static final int SAVE_DOCUMENT = 42;
    private Uri documentUri;
    private boolean documentIsSnapshot = true;
    private int documentGeneration;
    private final ExecutorService fileWorker = Executors.newSingleThreadExecutor();
    /** One caption + {@link ImageView} per rendered page, stacked. */
    private LinearLayout previewPages;
    private ScrollView previewScroll;
    private TextView previewError;
    private LinearLayout modifierPanel;
    private LinearLayout modifierRows;
    private EditText sourceField;
    private TextView mathHint;
    private boolean syncingCaret;
    private final java.util.ArrayList<RenderedPage> renderedPages = new java.util.ArrayList<>();
    private JSONArray documentMarkers = new JSONArray();
    private JSONArray documentModifierLinks = new JSONArray();
    private JSONArray documentCitations = new JSONArray();
    private JSONArray documentSegments = new JSONArray();
    private final java.util.LinkedHashMap<String, String[]> expandedSegments = new java.util.LinkedHashMap<>();
    private String modifierStart;
    private String modifierEnd;
    private int modifierBodyStart;
    private int modifierBodyEnd;
    private String inspectorSignature = "";
    private boolean editingModifier;
    private boolean inspectorOpen;
    private boolean wholeDocumentEditing;
    private int inspectorGeneration;
    private final ExecutorService inspectorRenderer = Executors.newSingleThreadExecutor();
    private final Runnable updateModifierComparisons = new Runnable() {
        @Override public void run() { requestModifierComparisons(); }
    };
    /** One-shot: scroll the first render to the first caption's layout y. */
    private boolean scrollToFirstCaption;
    /**
     * Desired scroll offset to reapply after replacing pages, or -1 when the
     * view owns the position. Retained when a shorter rebuild clamps the
     * view, so a later undo can return to it.
     */
    private int restoreScrollY = -1;
    private int visibleScrollY = -1;
    /** Width at which a restored pixel offset was recorded, or 0 for legacy saves. */
    private int restoreScrollWidth;
    /** True while we mutate the EditText ourselves (suppresses sync). */
    private boolean applying;

    /** Renders mathed off the UI thread (Typst layout is hundreds of ms). */
    private final ExecutorService renderer = Executors.newSingleThreadExecutor();
    private final Handler main = new Handler(Looper.getMainLooper());
    /** Bumped per edit; a worker result is dropped if a newer edit won. */
    private volatile int renderGeneration;
    /** A newer edit is waiting for the debounce timer. */
    private boolean renderPending;
    /** Pages currently stacked in the preview. */
    private int pagesLoaded;
    /** The document's full page count, from the latest chunk. */
    private int totalPages;
    /** One chunk in flight at a time (the renderer is single-threaded). */
    private boolean chunkInFlight;
    private String renderSnapshot;
    /**
     * The first paint shows the document itself. The editor opens with
     * its caret at 0, which reveal-on-caret would read as the first
     * statement's boundary and expand its raw `#1 … #2 \\app(…)` form.
     * Park the initial *render* caret at the document end (outside the
     * last statement for a newline-terminated document) so the preview
     * opens collapsed; the editor's own caret stays at 0.
     */
    private boolean firstRender = true;
    /** Debounce handle, so a burst of keystrokes rasterises once. */
    private final Runnable renderNow = new Runnable() {
        @Override
        public void run() {
            renderPending = false;
            String text = editor.getText().toString();
            boolean initial = firstRender;
            int caret = firstRender ? text.length() : editor.getSelectionStart();
            firstRender = false;
            startRender(text, caret, initial);
        }
    };

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);

        SharedPreferences prefs = getSharedPreferences(PREFS, MODE_PRIVATE);
        showMarkers = prefs.getBoolean(PREF_SHOW_MARKERS, false);
        inspectorOpen = showMarkers;
        String savedUri = prefs.getString("document_uri", null);
        if (savedUri != null) documentUri = Uri.parse(savedUri);
        documentIsSnapshot = prefs.getBoolean("document_snapshot", true);
        String initial = STARTER;
        String savedSnapshot = prefs.getString(PREF_SNAPSHOT, null);
        int savedCaret = prefs.getInt(PREF_CARET, -1);
        int savedScroll = prefs.getInt(PREF_SCROLL, -1);
        int savedScrollWidth = prefs.getInt(PREF_SCROLL_WIDTH, 0);
        boolean restoring = false;
        if (savedInstanceState != null) {
            String saved = savedInstanceState.getString(PREF_DOC);
            if (saved != null) {
                initial = saved;
                restoring = true;
            }
            savedCaret = savedInstanceState.getInt(PREF_CARET, savedCaret);
            savedScroll = savedInstanceState.getInt(PREF_SCROLL, savedScroll);
            savedScrollWidth = savedInstanceState.getInt(PREF_SCROLL_WIDTH, savedScrollWidth);
        } else {
            String saved = prefs.getString(PREF_DOC, null);
            if (saved != null) {
                initial = saved;
                restoring = true;
            }
        }

        buildUi();
        handle = restoring && savedSnapshot != null
                ? DocBridge.createFromSnapshot(savedSnapshot, initial) : 0;
        if (handle == 0) {
            handle = DocBridge.create(initial);
        }
        // Restore the caret in the model before the first state is applied.
        // Same text means this only moves the caret, so it adds no undo step.
        String state = restoring && savedCaret >= 0
                ? DocBridge.sync(handle, initial, savedCaret)
                : DocBridge.state(handle);
        applyState(state, true);
        if (restoring) {
            // A restored document opens where it was left, not at the first
            // caption. startRender keeps restoreScrollY and reapplies it
            // once the pages have been rebuilt.
            firstRender = false;
            if (savedScroll >= 0) {
                restoreScrollY = savedScroll;
                restoreScrollWidth = savedScrollWidth;
            }
            // The editor starts at the top; a long document's restored
            // caret would otherwise be off-screen. Wait for layout, then
            // bring the caret line into view without focusing (no IME).
            if (savedCaret >= 0) {
                final int caret = editor.getSelectionStart();
                editor.getViewTreeObserver().addOnGlobalLayoutListener(
                        new ViewTreeObserver.OnGlobalLayoutListener() {
                            @Override
                            public void onGlobalLayout() {
                                editor.getViewTreeObserver()
                                        .removeOnGlobalLayoutListener(this);
                                if (editor.getSelectionStart() == caret) {
                                    boolean moved = editor.bringPointIntoView(caret);
                                    trace("restoreCaretView", "caret", caret,
                                            "scrollY", editor.getScrollY(),
                                            "moved", moved);
                                }
                            }
                        });
            }
        }

        // The australVM Cranelift lane: one compile+execute roundtrip at
        // startup. The logcat line is the device-tier proof that the JIT
        // compiled, finalised (RW->RX over its arena) and ran code in
        // this process — AGENTS.md "#emthin-android".
        Log.i("emthin-pad", "cranelift roundtrip: " + DocBridge.jitRoundtrip(42));

        editor.addTextChangedListener(new TextWatcher() {
            private int markerOffset = -1;
            private int tabOffset = -1;

            @Override
            public void beforeTextChanged(CharSequence s, int st, int c, int a) {}

            @Override
            public void onTextChanged(CharSequence s, int st, int b, int c) {
                if (!applying) {
                    markerOffset = !pasting && c == 1 && s.charAt(st) == '#' ? st : -1;
                    tabOffset = b == 0 && c == 1 && s.charAt(st) == '\t' ? st : -1;
                }
            }

            @Override
            public void afterTextChanged(Editable s) {
                if (applying || handle == 0) {
                    return;
                }
                if (tabOffset >= 0) {
                    int at = tabOffset;
                    tabOffset = -1;
                    applying = true;
                    try {
                        s.delete(at, at + 1);
                        editor.setSelection(at);
                    } finally {
                        applying = false;
                    }
                    toggleMarkers();
                }
                if (markerOffset >= 0) {
                    int at = markerOffset;
                    markerOffset = -1;
                    String token = DocBridge.typedMarker(s.toString(), at);
                    if (!token.isEmpty()) {
                        applying = true;
                        try {
                            s.replace(at, at + 1, token);
                            editor.setSelection(at + token.length());
                        } finally {
                            applying = false;
                        }
                    }
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
            outState.putString(PREF_DOC, editor.getText().toString());
            outState.putInt(PREF_CARET, Math.max(0, editor.getSelectionStart()));
            outState.putInt(PREF_SCROLL, readingY());
            outState.putInt(PREF_SCROLL_WIDTH, readingWidth());
        }
    }

    @Override
    protected void onPause() {
        saveDocument();
        savePosition();
        super.onPause();
    }

    /** Writes the current editor text so a cold launch resumes the document. */
    private void saveDocument() {
        if (editor == null || handle == 0) {
            return;
        }
        getSharedPreferences(PREFS, MODE_PRIVATE).edit()
                .putString(PREF_DOC, editor.getText().toString())
                .putString(PREF_SNAPSHOT, DocBridge.snapshot(handle))
                .apply();
    }

    /**
     * Writes the caret and preview offset for the next launch. During a
     * rebuild the pages are briefly removed and the ScrollView clamps to
     * 0, so fall back to the retained target until the pages are back.
     */
    private void savePosition() {
        if (editor == null || previewScroll == null) {
            return;
        }
        getSharedPreferences(PREFS, MODE_PRIVATE).edit()
                .putInt(PREF_CARET, Math.max(0, editor.getSelectionStart()))
                .putInt(PREF_SCROLL, readingY())
                .putInt(PREF_SCROLL_WIDTH, readingWidth())
                .apply();
    }

    /** The live preview offset, or the retained target while pages rebuild. */
    private int readingY() {
        return previewPages.getChildCount() > 0
                ? previewScroll.getScrollY()
                : Math.max(0, restoreScrollY);
    }

    /** Width corresponding to readingY, including before the first rebuilt page. */
    private int readingWidth() {
        return previewPages.getChildCount() == 0 && restoreScrollWidth > 0
                ? restoreScrollWidth : previewPages.getWidth();
    }

    @Override
    protected void onDestroy() {
        main.removeCallbacks(renderNow);
        main.removeCallbacks(updateModifierComparisons);
        renderer.shutdown();
        inspectorRenderer.shutdown();
        fileWorker.shutdown();
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

        TextView newButton = shortcut("^N New", new Runnable() {
            @Override public void run() { confirmNewDocument(); }
        });
        undoButton = shortcut("^Z Undo", new Runnable() {
            @Override public void run() { applyState(DocBridge.undo(handle), true); }
        });
        redoButton = shortcut("^Y Redo", new Runnable() {
            @Override public void run() { applyState(DocBridge.redo(handle), true); }
        });
        markersButton = shortcut(showMarkers ? "Tab Hide markers" : "Tab Show markers", new Runnable() {
            @Override public void run() { toggleMarkers(); }
        });
        TextView close = shortcut("Esc Close", new Runnable() {
            @Override public void run() { closeInspector(); }
        });
        bar.addView(newButton);
        bar.addView(undoButton);
        bar.addView(redoButton);
        bar.addView(shortcut("^O Open", new Runnable() {
            @Override public void run() { chooseDocument(); }
        }));
        bar.addView(shortcut("^S Save", new Runnable() {
            @Override public void run() { saveToDocument(); }
        }));
        bar.addView(shortcut("^F Find", new Runnable() {
            @Override public void run() { openSearch(); }
        }));
        bar.addView(shortcut("^K Cites", new Runnable() {
            @Override public void run() { showCitations(0); }
        }));
        bar.addView(shortcut("^L Fold", new Runnable() {
            @Override public void run() { toggleFoldAt(editor.getSelectionStart(), false); }
        }));
        bar.addView(markersButton);
        bar.addView(close);
        HorizontalScrollView shortcuts = new HorizontalScrollView(this);
        shortcuts.setHorizontalScrollBarEnabled(false);
        shortcuts.addView(bar);
        root.addView(shortcuts, new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT));
        searchBar = new LinearLayout(this);
        searchBar.setOrientation(LinearLayout.HORIZONTAL);
        searchBar.setVisibility(View.GONE);
        searchField = new EditText(this);
        searchField.setSingleLine(true);
        searchField.setTextColor(Color.parseColor("#E6EDF3"));
        searchField.setHintTextColor(Color.parseColor("#8796A5"));
        searchField.setHint("Find · Enter accept · ^S next · ^R previous · Esc cancel");
        searchField.setContentDescription("Find in document");
        searchField.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS);
        searchField.setImeOptions(EditorInfo.IME_ACTION_DONE);
        searchField.addTextChangedListener(new TextWatcher() {
            @Override public void beforeTextChanged(CharSequence s, int start, int count, int after) {}
            @Override public void onTextChanged(CharSequence s, int start, int before, int count) {}
            @Override public void afterTextChanged(Editable s) { updateSearch(); }
        });
        searchField.setOnEditorActionListener(new TextView.OnEditorActionListener() {
            @Override public boolean onEditorAction(TextView v, int actionId, KeyEvent event) {
                if (actionId == EditorInfo.IME_ACTION_DONE) { closeSearch(false); return true; }
                return false;
            }
        });
        searchBar.addView(searchField, new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1));
        searchBar.addView(shortcut("^R Prev", new Runnable() {
            @Override public void run() { moveSearch(-1); }
        }));
        searchBar.addView(shortcut("^S Next", new Runnable() {
            @Override public void run() { moveSearch(1); }
        }));
        searchBar.addView(shortcut("Enter", new Runnable() {
            @Override public void run() { closeSearch(false); }
        }));
        searchCount = panelLabel("");
        searchBar.addView(searchCount);
        root.addView(searchBar);
        status = new TextView(this);
        status.setTextColor(Color.parseColor("#9BA7B4"));
        status.setTextSize(12f);
        root.addView(status);

        editor = new CaretEditText(this);
        editor.setTypeface(android.graphics.Typeface.MONOSPACE);
        // Markup, not prose: keep the IME from red-underlining \\app(...)
        // and numbers or autocorrecting tokens.
        editor.setInputType(InputType.TYPE_CLASS_TEXT
                | InputType.TYPE_TEXT_FLAG_MULTI_LINE
                | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS);
        editor.setTextColor(Color.parseColor("#E6EDF3"));
        editor.setHintTextColor(Color.parseColor("#4E5965"));
        editor.setHint("#1 caption #2 \\app(#1, #2, w, h)");
        editor.setBackground(null);
        editor.setPadding(0, dp(12), 0, 0);
        editor.setGravity(android.view.Gravity.TOP | android.view.Gravity.START);
        root.addView(editor, new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f));
        // The source buffer remains the editing model, but the document
        // itself is shown through the rendered, interactive page below.
        editor.setVisibility(View.GONE);

        modifierPanel = new LinearLayout(this);
        modifierPanel.setOrientation(LinearLayout.VERTICAL);
        modifierPanel.setVisibility(View.GONE);
        modifierPanel.setBackgroundColor(Color.parseColor("#1A2229"));
        modifierPanel.setPadding(dp(8), dp(4), dp(8), dp(4));
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
        root.addView(previewError, new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT));

        previewScroll = new ScrollView(this);
        // fillViewport(false) is load-bearing: with fillViewport(true) the
        // ScrollView re-measures previewBox with EXACTLY(viewport), so the
        // LinearLayout hands its children AT_MOST caps — adjustViewBounds
        // clamps the full-height page raster (1892 x 2674) to the ~428px
        // remainder and fitCenter squeezes text to sub-pixel (black screen).
        // Unclamped, the wrap-content image measures its true height and the
        // ScrollView simply scrolls.
        previewScroll.setFillViewport(false);
        previewScroll.setBackgroundColor(Color.parseColor("#14181D"));
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
                    requestChunk(editor.getText().toString(), pagesLoaded,
                            editor.getSelectionStart());
                }
            }
        });
        // A touch means the user owns the reading position now. Drop a
        // target retained from a render the view could not reach; otherwise
        // the next rebuild would jump back to it after the user scrolled.
        previewScroll.setOnTouchListener(new View.OnTouchListener() {
            @Override
            public boolean onTouch(View v, MotionEvent event) {
                if (event.getActionMasked() == MotionEvent.ACTION_DOWN) {
                    restoreScrollY = -1;
                    visibleScrollY = -1;
                    restoreScrollWidth = 0;
                }
                return false;
            }
        });
        root.addView(previewScroll, new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f));
        root.addView(modifierPanel, new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f));

        setContentView(root);
    }

    private int dp(int v) {
        return Math.round(v * getResources().getDisplayMetrics().density);
    }

    private void fileMessage(String text) {
        android.widget.Toast.makeText(this, text, android.widget.Toast.LENGTH_LONG).show();
    }

    private void chooseDocument() {
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        intent.setType("*/*");
        intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
        startActivityForResult(intent, OPEN_DOCUMENT);
    }

    private void saveToDocument() {
        if (handle == 0) return;
        if (documentUri == null) {
            Intent intent = new Intent(Intent.ACTION_CREATE_DOCUMENT);
            intent.addCategory(Intent.CATEGORY_OPENABLE);
            intent.setType("application/octet-stream");
            intent.putExtra(Intent.EXTRA_TITLE, "document.mathed");
            intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                    | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
            startActivityForResult(intent, SAVE_DOCUMENT);
        } else {
            writeDocument(documentUri, documentIsSnapshot);
        }
    }

    @Override protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (resultCode != RESULT_OK || data == null || data.getData() == null) return;
        final Uri uri = data.getData();
        int flags = data.getFlags() & (Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
        try { getContentResolver().takePersistableUriPermission(uri, flags); }
        catch (SecurityException e) { Log.w("emthin-pad", "document access is temporary", e); }
        if (requestCode == OPEN_DOCUMENT) readDocument(uri);
        else if (requestCode == SAVE_DOCUMENT) writeDocument(uri, true);
    }

    private void rememberDocument(Uri uri, boolean snapshot) {
        documentUri = uri;
        documentIsSnapshot = snapshot;
        getSharedPreferences(PREFS, MODE_PRIVATE).edit().putString("document_uri", uri.toString())
                .putBoolean("document_snapshot", snapshot).apply();
    }

    private void readDocument(final Uri uri) {
        final String original = DocBridge.snapshot(handle);
        fileWorker.execute(new Runnable() {
            @Override public void run() {
                try (java.io.InputStream input = getContentResolver().openInputStream(uri);
                     java.io.ByteArrayOutputStream out = new java.io.ByteArrayOutputStream()) {
                    if (input == null) throw new java.io.IOException("No document stream");
                    byte[] buffer = new byte[16384];
                    for (int n; (n = input.read(buffer)) != -1;) {
                        if (out.size() + n > 32 * 1024 * 1024) throw new java.io.IOException("Document exceeds 32 MB");
                        out.write(buffer, 0, n);
                    }
                    final byte[] bytes = out.toByteArray();
                    main.post(new Runnable() {
                        @Override public void run() {
                            if (handle == 0) return;
                            if (!original.equals(DocBridge.snapshot(handle))) {
                                fileMessage("Document changed while opening; open it again.");
                                return;
                            }
                            long next = DocBridge.openSnapshot(Base64.encodeToString(bytes, Base64.NO_WRAP));
                            boolean snapshot = next != 0;
                            if (!snapshot) {
                                try {
                                    String text = java.nio.charset.StandardCharsets.UTF_8.newDecoder()
                                            .decode(ByteBuffer.wrap(bytes)).toString();
                                    next = DocBridge.create(text);
                                } catch (java.nio.charset.CharacterCodingException e) {
                                    fileMessage("Could not open this document.");
                                    return;
                                }
                            }
                            if (inspectorOpen) closeInspector();
                            if (searchBar.getVisibility() == View.VISIBLE) closeSearch(false);
                            long old = handle;
                            handle = next;
                            DocBridge.free(old);
                            documentGeneration++;
                            expandedSegments.clear();
                            selectionAnchor = -1;
                            restoreScrollY = -1;
                            firstRender = true;
                            if (snapshot) {
                                rememberDocument(uri, true);
                            } else {
                                // A plain Typst file cannot hold Loro marks. Save it
                                // to a new .mathed snapshot instead of losing them.
                                documentUri = null;
                                documentIsSnapshot = true;
                                getSharedPreferences(PREFS, MODE_PRIVATE).edit()
                                        .remove("document_uri").remove("document_snapshot").apply();
                            }
                            applyState(DocBridge.state(handle), true);
                            fileMessage("Document opened");
                        }
                    });
                } catch (Exception e) {
                    main.post(new Runnable() { @Override public void run() { fileMessage("Could not open document: " + e.getMessage()); } });
                }
            }
        });
    }

    private void writeDocument(final Uri uri, final boolean snapshot) {
        final int generation = documentGeneration;
        final byte[] data = snapshot ? Base64.decode(DocBridge.snapshot(handle), Base64.DEFAULT)
                : editor.getText().toString().getBytes(java.nio.charset.StandardCharsets.UTF_8);
        fileWorker.execute(new Runnable() {
            @Override public void run() {
                try (java.io.OutputStream out = getContentResolver().openOutputStream(uri, "wt")) {
                    if (out == null) throw new java.io.IOException("No document stream");
                    out.write(data);
                    out.flush();
                    main.post(new Runnable() { @Override public void run() {
                        if (handle != 0 && documentGeneration == generation) rememberDocument(uri, snapshot);
                        fileMessage("Document saved");
                    } });
                } catch (Exception e) {
                    main.post(new Runnable() { @Override public void run() { fileMessage("Could not save document: " + e.getMessage()); } });
                }
            }
        });
    }

    private void confirmNewDocument() {
        new AlertDialog.Builder(this, android.R.style.Theme_Material_Dialog_Alert)
                .setTitle("New document")
                .setMessage("Replace the current text with the starter document?"
                        + " This can be undone.")
                .setPositiveButton("New", new DialogInterface.OnClickListener() {
                    @Override
                    public void onClick(DialogInterface dialog, int which) {
                        newDocument();
                    }
                })
                .setNegativeButton("Cancel", null)
                .show();
    }

    /** Replaces the text with the starter and reopens the preview at its caption. */
    private void newDocument() {
        if (handle == 0) {
            return;
        }
        documentUri = null;
        documentIsSnapshot = true;
        documentGeneration++;
        expandedSegments.clear();
        // A New document starts over instead of inheriting the old reading
        // position if the process dies before onPause saves the new one.
        getSharedPreferences(PREFS, MODE_PRIVATE).edit()
                .remove(PREF_CARET)
                .remove(PREF_SCROLL)
                .remove(PREF_SCROLL_WIDTH)
                .remove("document_uri")
                .remove("document_snapshot")
                .apply();
        firstRender = true;
        applyState(DocBridge.sync(handle, STARTER, 0), true);
        // Run the initial render now, so a quick Undo cannot inherit the
        // first-paint caret/scroll special case meant for this replacement.
        main.removeCallbacks(renderNow);
        renderNow.run();
    }

    /**
     * An {@link EditText} that re-renders the preview when the caret moves
     * (reveal-on-caret): a tap or arrow-key caret move does not change the
     * text, so no {@link TextWatcher} fires, but the preview must still
     * re-typeset to expand the token under the new caret.
     */
    private final class CaretEditText extends EditText {
        CaretEditText(android.content.Context context) {
            super(context);
        }

        @Override
        protected void onSelectionChanged(int selStart, int selEnd) {
            super.onSelectionChanged(selStart, selEnd);
            // `applying` guards the programmatic setText in applyState
            // (which schedules its own render); handle == 0 is the
            // pre-create window before the model exists.
            if (!applying && !syncingCaret && handle != 0) {
                scheduleRender();
                updateModifierPanel();
            }
            invalidateRenderedCaret();
        }
    }

    private final class SourceEditText extends EditText {
        SourceEditText(android.content.Context context) { super(context); }
        @Override protected void onSelectionChanged(int start, int end) {
            super.onSelectionChanged(start, end);
            if (this != sourceField || syncingCaret || editingModifier || !hasFocus()
                    || start < 0 || start != end || editor == null) return;
            setDocumentCaret(modifierBodyStart + start);
            updateMathHint();
        }
    }

    private void updateMathHint() {
        if (mathHint == null || sourceField == null || editor == null || handle == 0) return;
        String candidate = DocBridge.mathCompletion(editor.getText().toString(), editor.getSelectionStart());
        try {
            if (candidate.equals("null")) {
                mathHint.setVisibility(View.GONE);
            } else {
                mathHint.setText("↳ " + new JSONObject(candidate).getString("preview"));
                mathHint.setVisibility(View.VISIBLE);
            }
        } catch (Exception e) { mathHint.setVisibility(View.GONE); }
    }

    private void setDocumentCaret(int offset) {
        if (editor == null) return;
        int caret = Math.max(0, Math.min(editor.length(), offset));
        syncingCaret = true;
        try { editor.setSelection(caret); } finally { syncingCaret = false; }
        invalidateRenderedCaret();
        savePosition();
    }

    private void invalidateRenderedCaret() {
        for (RenderedPage page : renderedPages) page.invalidate();
    }

    private final class CaretSpot {
        RenderedPage page;
        JSONArray hit;
        boolean after;
        int distance = Integer.MAX_VALUE;
    }

    private CaretSpot caretSpot(int caret) {
        CaretSpot best = new CaretSpot();
        for (RenderedPage page : renderedPages) {
            for (int i = 0; i < page.hits.length(); i++) {
                JSONArray hit = page.hits.optJSONArray(i);
                if (hit == null) continue;
                int offset = hit.optInt(4, -1);
                if (offset < 0) continue;
                int distance = Math.abs(caret - offset);
                if (distance < best.distance) {
                    best.page = page;
                    best.hit = hit;
                    best.after = caret > offset;
                    best.distance = distance;
                }
            }
        }
        return best;
    }

    private final class RenderedPage extends ImageView {
        final JSONArray hits;
        final Paint caretPaint = new Paint(Paint.ANTI_ALIAS_FLAG);
        RenderedPage(android.content.Context context, JSONArray hits) {
            super(context);
            this.hits = hits;
            caretPaint.setColor(Color.parseColor("#B3CFE4"));
            caretPaint.setStyle(Paint.Style.STROKE);
            caretPaint.setStrokeWidth(Math.max(2f, getResources().getDisplayMetrics().density));
            setFocusable(true);
            setFocusableInTouchMode(true);
        }
        @Override protected void onDraw(Canvas canvas) {
            super.onDraw(canvas);
            if (editor == null) return;
            if (selectionAnchor >= 0 && selectionAnchor != editor.getSelectionStart()) {
                int begin = Math.min(selectionAnchor, editor.getSelectionStart());
                int end = Math.max(selectionAnchor, editor.getSelectionStart());
                Paint selected = new Paint();
                selected.setColor(0x88628AB3);
                float factor = getWidth() / PAGE_WIDTH_PT;
                for (int i = 0; i < hits.length(); i++) {
                    JSONArray hit = hits.optJSONArray(i);
                    if (hit == null) continue;
                    int pos = hit.optInt(4, -1);
                    if (pos >= begin && pos < end)
                        canvas.drawRect((float) hit.optDouble(0) * factor,
                                (float) hit.optDouble(1) * factor,
                                (float) hit.optDouble(2) * factor,
                                (float) hit.optDouble(3) * factor, selected);
                }
            }
            if (searchBar != null && searchBar.getVisibility() == View.VISIBLE) {
                Paint highlight = new Paint();
                float factor = getWidth() / PAGE_WIDTH_PT;
                for (int i = 0; i < hits.length(); i++) {
                    JSONArray hit = hits.optJSONArray(i);
                    if (hit == null) continue;
                    int pos = hit.optInt(4, -1);
                    int lo = 0, hi = searchMatches.length();
                    while (lo < hi) {
                        int mid = (lo + hi) >>> 1;
                        if (searchMatches.optJSONArray(mid).optInt(1) <= pos) lo = mid + 1;
                        else hi = mid;
                    }
                    if (lo < searchMatches.length()) {
                        JSONArray match = searchMatches.optJSONArray(lo);
                        if (match != null && pos >= match.optInt(0) && pos < match.optInt(1)) {
                            highlight.setColor(lo == searchCurrent ? 0x887DBB55 : 0x66898920);
                            canvas.drawRect((float) hit.optDouble(0) * factor,
                                    (float) hit.optDouble(1) * factor,
                                    (float) hit.optDouble(2) * factor,
                                    (float) hit.optDouble(3) * factor, highlight);
                        }
                    }
                }
            }
            CaretSpot spot = caretSpot(Math.max(0, editor.getSelectionStart()));
            if (spot.page != this) {
                if (spot.page == null && !renderedPages.isEmpty() && renderedPages.get(0) == this) {
                    float x = getWidth() * .1f, y = getWidth() * .1f;
                    canvas.drawRect(x, y, x + dp(10), y + dp(20), caretPaint);
                }
                return;
            }
            float scale = getWidth() / PAGE_WIDTH_PT;
            float left = (float) spot.hit.optDouble(0) * scale;
            float right = (float) spot.hit.optDouble(2) * scale;
            float width = Math.max(dp(6), Math.abs(right - left));
            if (spot.after) left = right;
            canvas.drawRect(left, (float) spot.hit.optDouble(1) * scale,
                    left + width, (float) spot.hit.optDouble(3) * scale, caretPaint);
        }
    }

    private void focusRenderedCaret() {
        CaretSpot spot = caretSpot(editor.getSelectionStart());
        RenderedPage page = spot.page != null ? spot.page
                : (renderedPages.isEmpty() ? null : renderedPages.get(0));
        if (page != null) {
            page.requestFocus();
            page.invalidate();
            page.post(new Runnable() {
                @Override public void run() {
                    CaretSpot current = caretSpot(editor.getSelectionStart());
                    if (current.page == null) return;
                    float scale = current.page.getWidth() / PAGE_WIDTH_PT;
                    int x = Math.round((float) current.hit.optDouble(0) * scale);
                    int top = Math.round((float) current.hit.optDouble(1) * scale);
                    int bottom = Math.round((float) current.hit.optDouble(3) * scale);
                    current.page.requestRectangleOnScreen(new android.graphics.Rect(
                            x, top, x + dp(12), bottom), false);
                }
            });
        }
    }

    private void moveRenderedCaret(int direction) {
        int current = editor.getSelectionStart();
        int candidate = direction > 0 ? Integer.MAX_VALUE : -1;
        for (RenderedPage page : renderedPages) {
            for (int i = 0; i < page.hits.length(); i++) {
                JSONArray hit = page.hits.optJSONArray(i);
                if (hit == null) continue;
                int offset = hit.optInt(4, -1);
                if (offset < 0 || offset > editor.length()) continue;
                if (direction > 0 && offset > current) candidate = Math.min(candidate, offset);
                if (direction < 0 && offset < current) candidate = Math.max(candidate, offset);
                if (offset < editor.length()) {
                    int end = offset + Character.charCount(Character.codePointAt(editor.getText(), offset));
                    if (direction > 0 && end > current) candidate = Math.min(candidate, end);
                    if (direction < 0 && end < current) candidate = Math.max(candidate, end);
                }
            }
        }
        if (candidate != Integer.MAX_VALUE && candidate >= 0) setDocumentCaret(candidate);
        focusRenderedCaret();
    }

    private void moveRenderedCaretVertical(int direction) {
        CaretSpot current = caretSpot(editor.getSelectionStart());
        if (current.page == null) return;
        float scale = current.page.getWidth() / PAGE_WIDTH_PT;
        float x = (float) current.hit.optDouble(0) * scale;
        float y = current.page.getTop() + (float) current.hit.optDouble(1) * scale;
        double best = Double.MAX_VALUE;
        int candidate = -1;
        for (RenderedPage page : renderedPages) {
            float pageScale = page.getWidth() / PAGE_WIDTH_PT;
            for (int i = 0; i < page.hits.length(); i++) {
                JSONArray hit = page.hits.optJSONArray(i);
                if (hit == null) continue;
                float targetY = page.getTop() + (float) hit.optDouble(1) * pageScale;
                float dy = (targetY - y) * direction;
                if (dy <= dp(2)) continue;
                float targetX = (float) hit.optDouble(0) * pageScale;
                double score = dy * 10000.0 + Math.abs(targetX - x);
                if (score < best) { best = score; candidate = hit.optInt(4, -1); }
            }
        }
        if (candidate >= 0) setDocumentCaret(candidate);
        focusRenderedCaret();
    }

    @Override
    public boolean dispatchKeyEvent(KeyEvent event) {
        if (event.getAction() == KeyEvent.ACTION_DOWN && searchBar != null
                && searchBar.getVisibility() == View.VISIBLE) {
            if (event.getKeyCode() == KeyEvent.KEYCODE_ESCAPE) { closeSearch(true); return true; }
            if (event.getKeyCode() == KeyEvent.KEYCODE_ENTER) { closeSearch(false); return true; }
            if (event.isCtrlPressed() && event.getKeyCode() == KeyEvent.KEYCODE_S) { moveSearch(1); return true; }
            if (event.isCtrlPressed() && event.getKeyCode() == KeyEvent.KEYCODE_R) { moveSearch(-1); return true; }
        }
        if (event.getAction() == KeyEvent.ACTION_DOWN && !event.isCtrlPressed() && !event.isAltPressed()) {
            int code = event.getKeyCode();
            if (code == KeyEvent.KEYCODE_DPAD_UP || code == KeyEvent.KEYCODE_DPAD_DOWN) {
                int direction = code == KeyEvent.KEYCODE_DPAD_UP ? -1 : 1;
                if (sourceField != null && sourceField.hasFocus() && sourceField.getLayout() != null
                        && sourceField.getSelectionStart() == sourceField.getSelectionEnd()) {
                    int line = sourceField.getLayout().getLineForOffset(sourceField.getSelectionStart());
                    if ((direction < 0 && line == 0)
                            || (direction > 0 && line == sourceField.getLayout().getLineCount() - 1)) {
                        closeInspector();
                        moveRenderedCaretVertical(direction);
                        return true;
                    }
                }
                if (getCurrentFocus() instanceof RenderedPage) {
                    if (event.isShiftPressed() && selectionAnchor < 0) selectionAnchor = editor.getSelectionStart();
                    if (!event.isShiftPressed()) selectionAnchor = -1;
                    moveRenderedCaretVertical(direction);
                    return true;
                }
            }
            if (code == KeyEvent.KEYCODE_DPAD_LEFT || code == KeyEvent.KEYCODE_DPAD_RIGHT) {
                int direction = code == KeyEvent.KEYCODE_DPAD_LEFT ? -1 : 1;
                if (sourceField != null && sourceField.hasFocus()
                        && sourceField.getSelectionStart() == sourceField.getSelectionEnd()
                        && ((direction < 0 && sourceField.getSelectionStart() == 0)
                            || (direction > 0 && sourceField.getSelectionStart() == sourceField.length()))) {
                    closeInspector();
                    moveRenderedCaret(direction);
                    return true;
                }
                if (getCurrentFocus() instanceof RenderedPage) {
                    if (event.isShiftPressed() && selectionAnchor < 0) selectionAnchor = editor.getSelectionStart();
                    if (!event.isShiftPressed() && selectionAnchor >= 0) {
                        int edge = direction < 0 ? Math.min(selectionAnchor, editor.getSelectionStart())
                                : Math.max(selectionAnchor, editor.getSelectionStart());
                        selectionAnchor = -1;
                        setDocumentCaret(edge);
                        focusRenderedCaret();
                        return true;
                    }
                    moveRenderedCaret(direction);
                    return true;
                }
            }
            if (getCurrentFocus() instanceof RenderedPage && event.getUnicodeChar() >= 32) {
                if (hasRenderedSelection()) deleteRenderedSelection();
                inspectorOpen = true;
                wholeDocumentEditing = false;
                inspectorSignature = "";
                updateModifierPanel();
                if (sourceField != null) {
                    focusSourceCaret();
                    sourceField.getText().insert(sourceField.getSelectionStart(),
                            new String(Character.toChars(event.getUnicodeChar())));
                    return true;
                }
            }
        }
        if (event.getAction() == KeyEvent.ACTION_DOWN && event.isCtrlPressed()
                && !event.isAltPressed() && getCurrentFocus() instanceof RenderedPage) {
            switch (event.getKeyCode()) {
                case KeyEvent.KEYCODE_A:
                    selectionAnchor = 0;
                    setDocumentCaret(editor.length());
                    return true;
                case KeyEvent.KEYCODE_C: copyRenderedSelection(); return true;
                case KeyEvent.KEYCODE_X: copyRenderedSelection(); deleteRenderedSelection(); return true;
                case KeyEvent.KEYCODE_V: pasteRenderedClipboard(); return true;
            }
        }
        if (event.getAction() == KeyEvent.ACTION_DOWN && event.isCtrlPressed()
                && !event.isAltPressed()
                && (event.getKeyCode() == KeyEvent.KEYCODE_DPAD_LEFT || event.getKeyCode() == KeyEvent.KEYCODE_DPAD_RIGHT)
                && (getCurrentFocus() instanceof RenderedPage || (!event.isShiftPressed() && sourceField != null && sourceField.hasFocus()))) {
            int direction = event.getKeyCode() == KeyEvent.KEYCODE_DPAD_LEFT ? -1 : 1;
            if (event.isShiftPressed() && selectionAnchor < 0) selectionAnchor = editor.getSelectionStart();
            if (!event.isShiftPressed()) selectionAnchor = -1;
            int target = DocBridge.wordBoundary(editor.getText().toString(), editor.getSelectionStart(), direction);
            if (sourceField != null && sourceField.hasFocus()
                    && target >= modifierBodyStart && target <= modifierBodyEnd) {
                sourceField.setSelection(target - modifierBodyStart);
            } else {
                if (inspectorOpen) closeInspector();
                setDocumentCaret(target);
                focusRenderedCaret();
            }
            return true;
        }
        if (event.getAction() == KeyEvent.ACTION_DOWN && event.getRepeatCount() == 0) {
            if (event.isCtrlPressed()) {
                switch (event.getKeyCode()) {
                    case KeyEvent.KEYCODE_N: confirmNewDocument(); return true;
                    case KeyEvent.KEYCODE_Z: applyState(DocBridge.undo(handle), true); return true;
                    case KeyEvent.KEYCODE_Y: applyState(DocBridge.redo(handle), true); return true;
                    case KeyEvent.KEYCODE_O: chooseDocument(); return true;
                    case KeyEvent.KEYCODE_S: saveToDocument(); return true;
                    case KeyEvent.KEYCODE_F: openSearch(); return true;
                    case KeyEvent.KEYCODE_R: if (event.isShiftPressed()) { openSearch(); return true; } break;
                    case KeyEvent.KEYCODE_K: showCitations(0); return true;
                    case KeyEvent.KEYCODE_L: toggleFoldAt(editor.getSelectionStart(), false); return true;
                }
                if (event.getKeyCode() >= KeyEvent.KEYCODE_1 && event.getKeyCode() <= KeyEvent.KEYCODE_9) {
                    showCitations(event.getKeyCode() - KeyEvent.KEYCODE_1 + 1);
                    return true;
                }
            } else if (event.getKeyCode() == KeyEvent.KEYCODE_ESCAPE && inspectorOpen) {
                closeInspector();
                return true;
            }
        }
        if (event.getKeyCode() == KeyEvent.KEYCODE_TAB && !event.isCtrlPressed()
                && !event.isAltPressed()) {
            if (event.getAction() == KeyEvent.ACTION_DOWN && event.getRepeatCount() == 0) {
                toggleMarkers();
            }
            return true;
        }
        return super.dispatchKeyEvent(event);
    }

    private void openSearch() {
        if (handle == 0) return;
        if (searchBar.getVisibility() != View.VISIBLE) {
            searchOrigin = editor.getSelectionStart();
            if (inspectorOpen) closeInspector();
            searchBar.setVisibility(View.VISIBLE);
            searchField.setText("");
            searchMatches = new JSONArray();
            searchCurrent = -1;
            searchCount.setText("");
        }
        searchField.requestFocus();
    }

    private void updateSearch() {
        if (handle == 0 || searchBar.getVisibility() != View.VISIBLE) return;
        try {
            searchMatches = new JSONArray(DocBridge.search(editor.getText().toString(), searchField.getText().toString()));
            searchCurrent = -1;
            for (int i = 0; i < searchMatches.length(); i++) {
                if (searchMatches.getJSONArray(i).getInt(0) >= searchOrigin) { searchCurrent = i; break; }
            }
            if (searchCurrent < 0 && searchMatches.length() > 0) searchCurrent = 0;
            showSearchMatch();
        } catch (Exception e) { searchCount.setText("Search unavailable"); }
    }

    private void moveSearch(int direction) {
        int count = searchMatches.length();
        if (count == 0) return;
        searchCurrent = (searchCurrent + direction + count) % count;
        showSearchMatch();
    }

    private void showSearchMatch() {
        if (searchCurrent < 0) {
            searchCount.setText(searchField.length() == 0 ? "" : "No matches");
        } else {
            JSONArray match = searchMatches.optJSONArray(searchCurrent);
            if (match != null) setDocumentCaret(match.optInt(0));
            searchCount.setText((searchCurrent + 1) + " / " + searchMatches.length());
            scrollSearchMatch();
        }
        invalidateRenderedCaret();
    }

    private void scrollSearchMatch() {
        if (searchCurrent < 0 || searchBar.getVisibility() != View.VISIBLE) return;
        final int target = searchMatches.optJSONArray(searchCurrent).optInt(0);
        CaretSpot spot = caretSpot(target);
        if (spot.page != null && spot.distance == 0) {
            final RenderedPage page = spot.page;
            final JSONArray hit = spot.hit;
            page.post(new Runnable() {
                @Override public void run() {
                    if (searchBar.getVisibility() != View.VISIBLE) return;
                    float scale = page.getWidth() / PAGE_WIDTH_PT;
                    int x = Math.round((float) hit.optDouble(0) * scale);
                    page.requestRectangleOnScreen(new android.graphics.Rect(x,
                            Math.round((float) hit.optDouble(1) * scale), x + dp(12),
                            Math.round((float) hit.optDouble(3) * scale)), false);
                }
            });
        } else if (!chunkInFlight && totalPages > pagesLoaded) {
            requestChunk(editor.getText().toString(), pagesLoaded, editor.getSelectionStart());
        }
    }

    private void closeSearch(boolean cancel) {
        if (cancel) setDocumentCaret(searchOrigin);
        searchBar.setVisibility(View.GONE);
        searchMatches = new JSONArray();
        searchCurrent = -1;
        android.view.inputmethod.InputMethodManager ime =
                (android.view.inputmethod.InputMethodManager) getSystemService(INPUT_METHOD_SERVICE);
        if (ime != null) ime.hideSoftInputFromWindow(searchField.getWindowToken(), 0);
        invalidateRenderedCaret();
        focusRenderedCaret();
    }

    private void focusSourceCaret() {
        if (sourceField == null) return;
        int offset = Math.max(0, Math.min(sourceField.length(), editor.getSelectionStart() - modifierBodyStart));
        syncingCaret = true;
        try {
            sourceField.setSelection(offset);
            sourceField.setShowSoftInputOnFocus(false);
            sourceField.requestFocus();
            sourceField.setShowSoftInputOnFocus(true);
        } finally { syncingCaret = false; }
        invalidateRenderedCaret();
        updateMathHint();
    }

    private int citationAt(int caret) {
        for (int i = 0; i < documentCitations.length(); i++) {
            JSONObject cite = documentCitations.optJSONObject(i);
            if (cite == null || caret < cite.optInt("start") || caret >= cite.optInt("end")) continue;
            JSONArray numbers = cite.optJSONArray("numbers");
            if (numbers != null && numbers.length() > 0) return numbers.optInt(0);
        }
        return 0;
    }

    /** Toggle a translator/template's derived rendering without changing the document. */
    private boolean toggleFoldAt(int caret, boolean titleOnly) {
        String text = editor.getText().toString();
        for (int i = 0; i < documentSegments.length(); i++) {
            JSONObject segment = documentSegments.optJSONObject(i);
            if (segment == null) continue;
            String kind = segment.optString("kind");
            if (!kind.equals("Translator") && !kind.equals("Template") && !kind.equals("Base")) continue;
            JSONArray span = segment.optJSONArray("span");
            if (span == null || span.length() < 2) continue;
            int start = byteToUtf16(text, span.optInt(0)), end = byteToUtf16(text, span.optInt(1));
            String first = segment.optString("startId"), last = segment.optString("endId");
            int markerStart = start;
            for (int j = 0; j < documentMarkers.length(); j++) {
                JSONObject marker = documentMarkers.optJSONObject(j);
                if (marker != null && first.equals(marker.optString("id"))) {
                    markerStart = marker.optInt("start", start);
                    break;
                }
            }
            if (titleOnly ? caret < markerStart || caret > start : caret < markerStart || caret > end) continue;
            String key = first + "\u0000" + last;
            if (expandedSegments.remove(key) == null) expandedSegments.put(key, new String[] {first, last});
            setDocumentCaret(start);
            scheduleRender();
            return true;
        }
        return false;
    }

    private void showCitations(int openNumber) {
        if (editor == null || handle == 0) return;
        final String text = editor.getText().toString();
        ScrollView scroll = new ScrollView(this);
        LinearLayout rows = new LinearLayout(this);
        rows.setOrientation(LinearLayout.VERTICAL);
        rows.setPadding(dp(12), dp(8), dp(12), dp(12));
        scroll.addView(rows);
        final AlertDialog dialog = new AlertDialog.Builder(this, android.R.style.Theme_Material_Dialog_Alert)
                .setTitle("Citations").setView(scroll).create();
        appendCitationRows(rows, text, dialog, 0, openNumber);
        dialog.show();
    }

    private void appendCitationRows(LinearLayout rows, final String text, final AlertDialog dialog,
                                    final int depth, int openNumber) {
        try {
            JSONArray citations = new JSONArray(DocBridge.citationIndex(text));
            if (citations.length() == 0 && depth == 0) rows.addView(panelLabel("No citations"));
            for (int i = 0; i < citations.length(); i++) {
                JSONObject cite = citations.getJSONObject(i);
                JSONArray numbers = cite.getJSONArray("numbers");
                for (int j = 0; j < numbers.length(); j++) {
                    final int number = numbers.getInt(j);
                    final TextView heading = panelLabel("▸ [" + number + "]");
                    heading.setTextColor(Color.parseColor("#B7D5EA"));
                    heading.setPadding(dp(8 + depth * 12), dp(8), dp(8), dp(8));
                    final LinearLayout details = new LinearLayout(this);
                    details.setOrientation(LinearLayout.VERTICAL);
                    details.setPadding(dp(16 + depth * 12), 0, dp(8), dp(8));
                    details.setVisibility(View.GONE);
                    heading.setOnClickListener(new View.OnClickListener() {
                        @Override public void onClick(View v) {
                            boolean open = details.getVisibility() != View.VISIBLE;
                            details.setVisibility(open ? View.VISIBLE : View.GONE);
                            heading.setText((open ? "▾ [" : "▸ [") + number + "]");
                            if (open && details.getChildCount() == 0) {
                                details.addView(panelLabel("Loading…"));
                                final float scale = Math.max(1f, Math.min(3f, getResources().getDisplayMetrics().density));
                                final float width = Math.max(60f, (getResources().getDisplayMetrics().widthPixels - dp(100)) / scale);
                                inspectorRenderer.execute(new Runnable() {
                                    @Override public void run() {
                                        final String result = DocBridge.citationDetail(text, number, width, scale);
                                        main.post(new Runnable() {
                                            @Override public void run() {
                                                if (!dialog.isShowing()) return;
                                                details.removeAllViews();
                                                try {
                                                    JSONObject detail = new JSONObject(result);
                                                    if (detail.has("error")) {
                                                        details.addView(panelLabel(detail.getString("error")));
                                                    } else if ("bibliography".equals(detail.optString("kind"))) {
                                                        details.addView(panelLabel(detail.optString("text")));
                                                        JSONArray entries = detail.optJSONArray("entries");
                                                        if (entries != null) {
                                                            for (int k = 0; k < entries.length(); k++)
                                                                details.addView(panelLabel(entries.optString(k)));
                                                        }
                                                        if (!detail.optBoolean("resolved"))
                                                            details.addView(panelLabel(detail.optString("errorDetail", "Citation could not be resolved")));
                                                    } else {
                                                        byte[] png = Base64.decode(detail.getString("png"), Base64.DEFAULT);
                                                        Bitmap bitmap = BitmapFactory.decodeByteArray(png, 0, png.length);
                                                        if (bitmap == null) throw new IllegalStateException("invalid citation image");
                                                        bitmap.setDensity(Bitmap.DENSITY_NONE);
                                                        ImageView image = new ImageView(MainActivity.this);
                                                        image.setImageBitmap(bitmap);
                                                        image.setScaleType(ImageView.ScaleType.FIT_START);
                                                        image.setContentDescription("Referenced document text");
                                                        details.addView(image, new LinearLayout.LayoutParams(
                                                                LinearLayout.LayoutParams.MATCH_PARENT, bitmap.getHeight()));
                                                        if (depth < 7) appendCitationRows(details, detail.optString("text"), dialog, depth + 1, 0);
                                                    }
                                                } catch (Exception e) { details.addView(panelLabel("Could not display citation")); }
                                            }
                                        });
                                    }
                                });
                            }
                        }
                    });
                    rows.addView(heading);
                    rows.addView(details);
                    if (number == openNumber) heading.performClick();
                }
            }
        } catch (Exception e) { rows.addView(panelLabel("Could not list citations")); }
    }

    private boolean hasRenderedSelection() {
        return selectionAnchor >= 0 && selectionAnchor != editor.getSelectionStart();
    }

    private void copyRenderedSelection() {
        if (!hasRenderedSelection()) return;
        int begin = Math.min(selectionAnchor, editor.getSelectionStart());
        int end = Math.max(selectionAnchor, editor.getSelectionStart());
        String text = editor.getText().toString();
        StringBuilder visible = new StringBuilder();
        if (showMarkers) visible.append(text, begin, end);
        int at = begin;
        for (int i = 0; !showMarkers && i < documentMarkers.length(); i++) {
            JSONObject marker = documentMarkers.optJSONObject(i);
            if (marker == null) continue;
            int left = marker.optInt("start", -1), right = marker.optInt("end", -1);
            if (right <= at || left >= end || left < 0) continue;
            if (left > at) visible.append(text, at, Math.min(left, end));
            at = Math.min(end, Math.max(at, right));
        }
        if (!showMarkers && at < end) visible.append(text, at, end);
        android.content.ClipboardManager clipboard =
                (android.content.ClipboardManager) getSystemService(CLIPBOARD_SERVICE);
        if (clipboard != null) clipboard.setPrimaryClip(android.content.ClipData.newPlainText("", visible.toString()));
    }

    private void deleteRenderedSelection() {
        if (!hasRenderedSelection()) return;
        int begin = Math.min(selectionAnchor, editor.getSelectionStart());
        int end = Math.max(selectionAnchor, editor.getSelectionStart());
        selectionAnchor = -1;
        StringBuilder preservedMarkers = new StringBuilder();
        if (!showMarkers) {
            for (int i = 0; i < documentMarkers.length(); i++) {
                JSONObject marker = documentMarkers.optJSONObject(i);
                if (marker == null) continue;
                int left = Math.max(begin, marker.optInt("start", -1));
                int right = Math.min(end, marker.optInt("end", -1));
                if (left >= begin && right > left) preservedMarkers.append(editor.getText(), left, right);
            }
        }
        editor.getText().replace(begin, end, preservedMarkers);
        setDocumentCaret(begin);
    }

    private void pasteRenderedClipboard() {
        android.content.ClipboardManager clipboard =
                (android.content.ClipboardManager) getSystemService(CLIPBOARD_SERVICE);
        if (clipboard == null || !clipboard.hasPrimaryClip() || clipboard.getPrimaryClip() == null
                || clipboard.getPrimaryClip().getItemCount() == 0) return;
        CharSequence value = clipboard.getPrimaryClip().getItemAt(0).coerceToText(this);
        if (value == null) return;
        if (hasRenderedSelection()) deleteRenderedSelection();
        int caret = editor.getSelectionStart();
        pasting = true;
        try { editor.getText().insert(caret, value); } finally { pasting = false; }
        setDocumentCaret(caret + value.length());
    }

    private TextView shortcut(String label, final Runnable action) {
        TextView view = new TextView(this);
        view.setText(label);
        view.setTextColor(Color.parseColor("#AFC8D9"));
        view.setTextSize(13f);
        view.setPadding(dp(8), dp(10), dp(8), dp(10));
        view.setOnClickListener(new View.OnClickListener() {
            @Override public void onClick(View v) { action.run(); }
        });
        return view;
    }

    private final class TargetEditText extends EditText {
        TargetEditText(android.content.Context context) { super(context); }
        @Override protected void onSelectionChanged(int start, int end) {
            super.onSelectionChanged(start, end);
            if (this == targetsField) scheduleTargetCommit(this);
        }
    }

    private void scheduleTargetCommit(final EditText field) {
        field.post(new Runnable() {
            @Override public void run() {
                if (field == targetsField) commitTargetPairs(field, field.hasFocus());
            }
        });
    }

    /** Apply complete pairs outside the pair currently being edited. */
    private void commitTargetPairs(EditText field, boolean preserveActive) {
        if (field != targetsField || modifierStart == null || editingModifier) return;
        String value = field.getText().toString();
        String[] pairs = value.split(",", -1);
        String[] previous = appliedTargetList.split(",", -1);
        int cursor = Math.max(0, field.getSelectionStart());
        int active = -1;
        if (preserveActive) {
            active = 0;
            for (int i = 0; i < Math.min(cursor, value.length()); i++) {
                if (value.charAt(i) == ',') active++;
            }
        }
        java.util.ArrayList<String> complete = new java.util.ArrayList<>();
        for (int i = 0; i < pairs.length; i++) {
            String pair = i == active ? (i < previous.length ? previous[i] : "") : pairs[i];
            if (!pair.trim().isEmpty()) complete.add(pair.trim());
        }
        String next = android.text.TextUtils.join(", ", complete);
        if (next.equals(appliedTargetList)) return;
        String result = DocBridge.setMarkerTargets(handle, modifierStart, modifierEnd, next);
        try {
            JSONObject state = new JSONObject(result);
            if (state.has("error")) {
                field.setError(state.getString("error"));
                return;
            }
            field.setError(null);
            appliedTargetList = next;
            editingModifier = true;
            try { applyState(result, false); } finally { editingModifier = false; }
            scheduleModifierComparisons();
        } catch (Exception e) {
            field.setError("Could not update marker pairs");
        }
    }

    private void toggleMarkers() {
        showMarkers = !showMarkers;
        getSharedPreferences(PREFS, MODE_PRIVATE).edit().putBoolean(PREF_SHOW_MARKERS, showMarkers).apply();
        if (markersButton != null) {
            markersButton.setText(showMarkers ? "Tab Hide markers" : "Tab Show markers");
        }
        trace("markerVisibility", "shown", showMarkers);
        updateMarkerVisibility();
        updateSourceMarkerVisibility();
        if (editingModifier) {
            main.post(new Runnable() { @Override public void run() { syncColumnsWithMarkers(); } });
        } else {
            syncColumnsWithMarkers();
        }
        if (handle != 0) scheduleRender();
    }

    private void syncColumnsWithMarkers() {
        if (handle == 0) return;
        if (showMarkers) {
            if (!inspectorOpen) {
                inspectorOpen = true;
                wholeDocumentEditing = false;
                inspectorSignature = "";
                updateModifierPanel();
                focusSourceCaret();
            }
        } else if (inspectorOpen) {
            closeInspector();
        }
    }

    /** Hides marker glyphs without changing the editable document text. */
    private static final class HiddenMarker extends ReplacementSpan {
        @Override public int getSize(Paint paint, CharSequence text, int start, int end,
                                     Paint.FontMetricsInt fm) { return 0; }
        @Override public void draw(Canvas canvas, CharSequence text, int start, int end,
                                   float x, int top, int y, int bottom, Paint paint) {}
    }

    private void updateMarkerVisibility() {
        if (editor == null) return;
        Editable text = editor.getText();
        for (HiddenMarker span : text.getSpans(0, text.length(), HiddenMarker.class)) text.removeSpan(span);
        if (showMarkers) return;
        for (int i = 0; i < documentMarkers.length(); i++) {
            JSONObject marker = documentMarkers.optJSONObject(i);
            if (marker == null) continue;
            int start = marker.optInt("start", -1), end = marker.optInt("end", -1);
            if (start >= 0 && start < end && end <= text.length()) {
                text.setSpan(new HiddenMarker(), start, end, Spannable.SPAN_EXCLUSIVE_EXCLUSIVE);
            }
        }
    }

    private void updateSourceMarkerVisibility() {
        if (sourceField == null) return;
        Editable text = sourceField.getText();
        for (HiddenMarker span : text.getSpans(0, text.length(), HiddenMarker.class)) text.removeSpan(span);
        if (showMarkers) return;
        for (int i = 0; i < documentMarkers.length(); i++) {
            JSONObject marker = documentMarkers.optJSONObject(i);
            if (marker == null) continue;
            int start = marker.optInt("start", -1) - modifierBodyStart;
            int end = marker.optInt("end", -1) - modifierBodyStart;
            if (start >= 0 && start < end && end <= text.length()) {
                text.setSpan(new HiddenMarker(), start, end, Spannable.SPAN_EXCLUSIVE_EXCLUSIVE);
            }
        }
    }

    /** The adjacent markers around the caret identify the editable text part. */
    private void updateModifierPanel() {
        if (modifierPanel == null || editor == null || handle == 0 || editingModifier) return;
        if (!inspectorOpen) {
            modifierPanel.setVisibility(View.GONE);
            return;
        }
        int caret = editor.getSelectionStart();
        String startId = null, endId = null;
        int start = 0, end = 0;
        try {
            for (int i = 0; !wholeDocumentEditing && i + 1 < documentMarkers.length(); i++) {
                JSONObject a = documentMarkers.getJSONObject(i);
                JSONObject b = documentMarkers.getJSONObject(i + 1);
                boolean atCaret = editor.getSelectionStart() == editor.getSelectionEnd()
                        && caret >= a.getInt("end") && caret <= b.getInt("start");
                if (atCaret) {
                    startId = a.getString("id"); endId = b.getString("id");
                    start = a.getInt("end"); end = b.getInt("start");
                    break;
                }
            }
        } catch (Exception e) {
            Log.w("emthin-pad", "could not inspect markers", e);
        }
        if (startId == null) {
            String text = editor.getText().toString();
            caret = Math.max(0, Math.min(caret, text.length()));
            if (wholeDocumentEditing) {
                start = 0;
                end = text.length();
            } else {
                int previousNewline = text.lastIndexOf('\n', caret - 1);
                start = previousNewline < 0 ? 0 : previousNewline + 1;
                int nextNewline = text.indexOf('\n', caret);
                end = nextNewline < 0 ? text.length() : nextNewline;
                for (int i = 0; i < documentMarkers.length(); i++) {
                    JSONObject marker = documentMarkers.optJSONObject(i);
                    if (marker == null) continue;
                    int a = marker.optInt("start", -1), b = marker.optInt("end", -1);
                    if (b >= 0 && b <= caret) start = Math.max(start, b);
                    if (a >= caret) end = Math.min(end, a);
                }
                if (end < start) end = start;
            }
        }
        modifierStart = startId; modifierEnd = endId;
        modifierBodyStart = start; modifierBodyEnd = end;
        final boolean markedPart = startId != null;
        String targetList = markedPart ? markerPairsFor(startId, endId) : "";
        String signature = startId + "\n" + endId + "\n" + start + ":" + end
                + "\n" + editor.getText().subSequence(start, end) + "\n" + targetList;
        if (signature.equals(inspectorSignature)) return;
        inspectorSignature = signature;
        modifierPanel.removeAllViews();
        modifierPanel.setVisibility(View.VISIBLE);

        LinearLayout controls = new LinearLayout(this);
        controls.setOrientation(LinearLayout.HORIZONTAL);
        modifierPanel.addView(controls);
        mathHint = panelLabel("");
        mathHint.setVisibility(View.GONE);
        modifierPanel.addView(mathHint);

        final EditText source = new SourceEditText(this);
        sourceField = source;
        source.setSingleLine(false);
        source.setTextColor(Color.parseColor("#E6EDF3"));
        source.setTextSize(14f);
        GradientDrawable blockCaret = new GradientDrawable();
        blockCaret.setColor(Color.parseColor("#80B3CFE4"));
        blockCaret.setSize(dp(8), dp(20));
        source.setTextCursorDrawable(blockCaret);
        source.setText(editor.getText().subSequence(start, end).toString());
        source.setContentDescription("Editable text without modifiers");
        source.addTextChangedListener(new TextWatcher() {
            private int completionAt = -1;
            @Override public void beforeTextChanged(CharSequence s, int st, int c, int a) { completionAt = -1; }
            @Override public void onTextChanged(CharSequence s, int st, int b, int c) {
                if (!editingModifier && b == 0 && c == 1) {
                    char ch = s.charAt(st);
                    boolean run = Character.isLetterOrDigit(ch) && ch < 128
                            || "\\-><=:|!_~".indexOf(ch) >= 0;
                    if (!run) completionAt = st;
                }
            }
            @Override public void afterTextChanged(Editable s) {
                if (editingModifier || modifierBodyEnd > editor.length()) return;
                editingModifier = true;
                try {
                    int at = modifierBodyStart;
                    String replacement = s.toString();
                    if (completionAt >= 0) {
                        String candidate = DocBridge.mathCompletion(editor.getText().toString(), at + completionAt);
                        if (!candidate.equals("null")) {
                            try {
                                JSONObject completion = new JSONObject(candidate);
                                int start = completion.getInt("start") - at;
                                int end = completion.getInt("end") - at;
                                if (start >= 0 && end == completionAt && end <= replacement.length()) {
                                    String glyph = completion.getString("text");
                                    replacement = replacement.substring(0, start) + glyph + replacement.substring(end);
                                    int caret = start + glyph.length() + 1;
                                    source.setText(replacement);
                                    source.setSelection(Math.min(caret, source.length()));
                                }
                            } catch (Exception e) { Log.w("emthin-pad", "could not complete math input", e); }
                        }
                        completionAt = -1;
                    }
                    // A marker identifier is followed by alphanumerics, so
                    // keep a separator when replacing the whole body with a
                    // word; otherwise #i + bold becomes the marker #ibold.
                    if (markedPart && !replacement.isEmpty() && Character.isLetterOrDigit(replacement.codePointAt(0))) {
                        replacement = " " + replacement;
                    }
                    String old = editor.getText().subSequence(at, modifierBodyEnd).toString();
                    int prefix = 0;
                    int limit = Math.min(old.length(), replacement.length());
                    while (prefix < limit && old.charAt(prefix) == replacement.charAt(prefix)) prefix++;
                    if (prefix > 0 && prefix < old.length() && Character.isLowSurrogate(old.charAt(prefix))) prefix--;
                    if (prefix > 0 && prefix < replacement.length() && Character.isLowSurrogate(replacement.charAt(prefix))) prefix--;
                    int suffix = 0;
                    while (suffix < old.length() - prefix && suffix < replacement.length() - prefix
                            && old.charAt(old.length() - 1 - suffix) == replacement.charAt(replacement.length() - 1 - suffix)) suffix++;
                    while (suffix > 0 && ((old.length() - suffix < old.length()
                            && Character.isLowSurrogate(old.charAt(old.length() - suffix)))
                            || (replacement.length() - suffix < replacement.length()
                            && Character.isLowSurrogate(replacement.charAt(replacement.length() - suffix))))) suffix--;
                    int before = editor.length();
                    editor.getText().replace(at + prefix, at + old.length() - suffix,
                            replacement.substring(prefix, replacement.length() - suffix));
                    modifierBodyEnd += editor.length() - before;
                    String actual = editor.getText().subSequence(at, modifierBodyEnd).toString();
                    if (!actual.contentEquals(source.getText())) {
                        int cursor = Math.max(0, source.getSelectionStart());
                        int delta = actual.length() - source.length();
                        source.setText(actual);
                        source.setSelection(Math.max(0, Math.min(actual.length(), cursor + delta)));
                    }
                    inspectorSignature = "";
                } finally {
                    editingModifier = false;
                }
                updateSourceMarkerVisibility();
                scheduleModifierComparisons();
                source.post(new Runnable() {
                    @Override public void run() {
                        if (source == sourceField && source.hasFocus() && source.getSelectionStart() >= 0)
                            setDocumentCaret(modifierBodyStart + source.getSelectionStart());
                        updateMathHint();
                    }
                });
            }
        });
        controls.addView(source, new LinearLayout.LayoutParams(
                0, LinearLayout.LayoutParams.WRAP_CONTENT, 1));
        updateSourceMarkerVisibility();

        if (!markedPart) {
            targetsField = null;
            modifierRows = null;
            inspectorGeneration++;
            return;
        }

        final EditText targets = new TargetEditText(this);
        targetsField = targets;
        appliedTargetList = targetList;
        targets.setSingleLine(true);
        targets.setTextColor(Color.parseColor("#E6EDF3"));
        targets.setHintTextColor(Color.parseColor("#8796A5"));
        targets.setTextSize(14f);
        targets.setHint("a at, be e");
        targets.setContentDescription("Target marker pairs");
        targets.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS);
        targets.setImeOptions(EditorInfo.IME_ACTION_DONE);
        targets.setText(targetList);
        for (int i = 0; i < documentModifierLinks.length(); i++) {
            JSONObject link = documentModifierLinks.optJSONObject(i);
            if (link != null && modifierStart.equals(link.optString("startId"))
                    && modifierEnd.equals(link.optString("endId"))
                    && link.optJSONArray("targetIds") != null && !link.optBoolean("resolved", true)) {
                targets.setError("A marker pair no longer resolves; edit the list to repair it.");
                break;
            }
        }
        controls.addView(targets, new LinearLayout.LayoutParams(
                0, LinearLayout.LayoutParams.WRAP_CONTENT, 1));

        targets.addTextChangedListener(new TextWatcher() {
            @Override public void beforeTextChanged(CharSequence s, int start, int count, int after) {}
            @Override public void onTextChanged(CharSequence s, int start, int before, int count) {}
            @Override public void afterTextChanged(Editable s) { scheduleTargetCommit(targets); }
        });
        targets.setOnFocusChangeListener(new View.OnFocusChangeListener() {
            @Override public void onFocusChange(View v, boolean focused) {
                if (!focused) commitTargetPairs(targets, false);
            }
        });
        targets.setOnEditorActionListener(new TextView.OnEditorActionListener() {
            @Override public boolean onEditorAction(TextView v, int actionId, KeyEvent event) {
                if (actionId == EditorInfo.IME_ACTION_DONE) {
                    commitTargetPairs(targets, false);
                    targets.clearFocus();
                    return true;
                }
                return false;
            }
        });

        ScrollView scroll = new ScrollView(this);
        modifierRows = new LinearLayout(this);
        modifierRows.setOrientation(LinearLayout.VERTICAL);
        scroll.addView(modifierRows);
        modifierPanel.addView(scroll, new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f));
        scheduleModifierComparisons();
    }

    private void closeInspector() {
        if (targetsField != null) commitTargetPairs(targetsField, false);
        targetsField = null;
        inspectorOpen = false;
        wholeDocumentEditing = false;
        modifierStart = modifierEnd = null;
        sourceField = null;
        mathHint = null;
        inspectorSignature = "";
        inspectorGeneration++;
        main.removeCallbacks(updateModifierComparisons);
        modifierPanel.setVisibility(View.GONE);
        android.view.inputmethod.InputMethodManager ime =
                (android.view.inputmethod.InputMethodManager) getSystemService(INPUT_METHOD_SERVICE);
        if (ime != null) ime.hideSoftInputFromWindow(modifierPanel.getWindowToken(), 0);
        modifierPanel.clearFocus();
        focusRenderedCaret();
    }

    private String markerPairsFor(String start, String end) {
        java.util.LinkedHashSet<String> pairs = new java.util.LinkedHashSet<>();
        try {
            for (int i = 0; i < documentModifierLinks.length(); i++) {
                JSONObject link = documentModifierLinks.getJSONObject(i);
                JSONArray ids = link.optJSONArray("targetIds");
                if (ids != null && start.equals(link.optString("startId"))
                        && end.equals(link.optString("endId"))) {
                    pairs.add(ids.getString(0) + " " + ids.getString(1));
                }
            }
        } catch (Exception e) {
            Log.w("emthin-pad", "could not display marker pairs", e);
        }
        return android.text.TextUtils.join(", ", pairs);
    }

    private void scheduleModifierComparisons() {
        inspectorGeneration++;
        main.removeCallbacks(updateModifierComparisons);
        if (modifierStart != null && modifierEnd != null) main.postDelayed(updateModifierComparisons, 300);
    }

    private void requestModifierComparisons() {
        if (handle == 0 || modifierStart == null || modifierEnd == null || modifierRows == null) return;
        final int generation = inspectorGeneration;
        final String text = editor.getText().toString();
        final String snapshot = DocBridge.snapshot(handle);
        final String start = modifierStart, end = modifierEnd;
        final float scale = Math.max(1f, Math.min(3f, getResources().getDisplayMetrics().density));
        final float width = Math.max(60f, modifierRows.getWidth() / (2f * scale));
        inspectorRenderer.execute(new Runnable() {
            @Override public void run() {
                final String result = DocBridge.modifierComparisons(text, snapshot, start, end, width, scale);
                main.post(new Runnable() {
                    @Override public void run() {
                        if (generation == inspectorGeneration && modifierRows != null) showModifierComparisons(result);
                    }
                });
            }
        });
    }

    private TextView panelLabel(String text) {
        TextView label = new TextView(this);
        label.setText(text);
        label.setTextColor(Color.parseColor("#B7C4D0"));
        label.setPadding(dp(4), dp(4), dp(4), dp(4));
        return label;
    }

    private View comparisonImage(JSONObject image) {
        if (image == null || image.has("error")) return panelLabel("Preview unavailable");
        try {
            int w = image.getInt("width"), h = image.getInt("height");
            byte[] rgba = Base64.decode(image.getString("rgba"), Base64.DEFAULT);
            if (w <= 0 || h <= 0 || (long) w * h * 4 != rgba.length) return panelLabel("Preview unavailable");
            Bitmap bitmap = Bitmap.createBitmap(w, h, Bitmap.Config.ARGB_8888);
            bitmap.setDensity(Bitmap.DENSITY_NONE);
            bitmap.copyPixelsFromBuffer(ByteBuffer.wrap(rgba));
            ImageView view = new ImageView(this);
            view.setImageBitmap(bitmap);
            view.setScaleType(ImageView.ScaleType.FIT_START);
            view.setTag(h);
            view.setContentDescription("Rendered comparison");
            return view;
        } catch (Exception e) {
            return panelLabel("Preview unavailable");
        }
    }

    private void comparisonRow(View left, View right) {
        LinearLayout row = new LinearLayout(this);
        row.setOrientation(LinearLayout.HORIZONTAL);
        row.addView(left, new LinearLayout.LayoutParams(0,
                left instanceof ImageView ? (Integer) left.getTag() : LinearLayout.LayoutParams.WRAP_CONTENT, 1));
        row.addView(right, new LinearLayout.LayoutParams(0,
                right instanceof ImageView ? (Integer) right.getTag() : LinearLayout.LayoutParams.WRAP_CONTENT, 1));
        modifierRows.addView(row);
    }

    private void showModifierComparisons(String json) {
        modifierRows.removeAllViews();
        try {
            JSONObject result = new JSONObject(json);
            if (result.has("error")) { modifierRows.addView(panelLabel(result.getString("error"))); return; }
            JSONArray incoming = result.getJSONArray("incoming");
            comparisonRow(panelLabel("Modifiers"), panelLabel("Without modifier"));
            if (incoming.length() == 0) comparisonRow(panelLabel("None"), panelLabel(""));
            for (int i = 0; i < incoming.length(); i++) {
                JSONObject row = incoming.getJSONObject(i);
                comparisonRow(panelLabel(row.getString("label")), comparisonImage(row.optJSONObject("without")));
            }
            JSONArray outgoing = result.getJSONArray("outgoing");
            comparisonRow(panelLabel("Modified text"), panelLabel("Without this modifier"));
            if (outgoing.length() == 0) comparisonRow(panelLabel("No applications yet"), panelLabel(""));
            for (int i = 0; i < outgoing.length(); i++) {
                JSONObject row = outgoing.getJSONObject(i);
                comparisonRow(comparisonImage(row.optJSONObject("with")), comparisonImage(row.optJSONObject("without")));
                if (!row.optBoolean("legacy", false)) continue;
                final int start = row.getInt("start"), end = row.getInt("end");
                TextView remove = shortcut("Remove this application", new Runnable() {
                    @Override public void run() {
                        String state = DocBridge.setModifier(handle, modifierStart, modifierEnd, start, end, 0);
                        if (!state.contains("\"error\"")) {
                            applyState(state, false);
                            scheduleModifierComparisons();
                        }
                    }
                });
                modifierRows.addView(remove);
            }
        } catch (Exception e) {
            modifierRows.addView(panelLabel("Could not display comparisons"));
        }
    }

    // ----- mathed rendering --------------------------------------------

    /** Debounces: a burst of keystrokes rasterises once, ~300 ms after the last. */
    private void scheduleRender() {
        // Invalidate immediately, not when the debounce expires: a render
        // already on the worker may otherwise publish an obsolete page.
        renderGeneration++;
        renderPending = true;
        main.removeCallbacks(renderNow);
        main.postDelayed(renderNow, 300);
    }

    // Round-6 agent feedback: D-level render/state trace so the device
    // loop can see what the UI thread actually did (logcat -s emthin-pad:D).
    private void trace(String what, Object... kv) {
        StringBuilder b = new StringBuilder(what);
        for (Object o : kv) { b.append(' ').append(o); }
        android.util.Log.d("emthin-pad", b.toString());
    }

    /**
     * Starts a fresh page-frame render of {@code text}: the stacked
     * pages are dropped and chunk 0 streams in, unless a newer edit
     * supersedes the request while Typst is laying out.
     *
     * @param openAtCaption true for the first paint: scroll to the first
     *                      rendered caption page instead of leaving the
     *                      page margin at the top of the preview.
     */
    private void startRender(final String text, final int caret, final boolean openAtCaption) {
        visibleScrollY = -1;
        renderSnapshot = DocBridge.snapshot(handle);
        if (restoreScrollY >= 0 && restoreScrollWidth > 0 && previewPages.getWidth() > 0) {
            restoreScrollY = Math.round((float) restoreScrollY * previewPages.getWidth()
                    / restoreScrollWidth);
            restoreScrollWidth = 0;
        }
        renderGeneration++;
        pagesLoaded = 0;
        totalPages = 0;
        chunkInFlight = false;
        scrollToFirstCaption = openAtCaption;
        // Dropping the pages clamps the ScrollView to 0. Keep the reading
        // position across the rebuild; the first paint has no position yet
        // and scrolls to the caption instead. A retained target means the
        // previous render was too short to reach it (page count shrank) —
        // keep the original reading position, not the clamped one, so an
        // undo that restores the pages can return to it.
        if (openAtCaption) {
            restoreScrollY = -1;
            restoreScrollWidth = 0;
        } else if (restoreScrollY < 0) {
            restoreScrollY = previewScroll.getScrollY();
        }
        releasePageBitmaps();
        trace("startRender", "gen", renderGeneration, "textLen", text.length(),
                "caret", caret,
                "head", text.substring(0, Math.min(24, text.length())));
        requestChunk(text, 0, caret);
    }

    /**
     * Renders pages {@code start..start + CHUNK} on the worker thread
     * and stacks them only if no newer edit superseded the request. At
     * most one chunk is in flight (the renderer is single-threaded), so
     * chunks always append in order.
     */
    private void requestChunk(final String text, final int start, final int caret) {
        if (renderPending || chunkInFlight || (totalPages > 0 && start >= totalPages)) {
            return; // one in flight already, or nothing left to load
        }
        final int generation = renderGeneration;
        final String snapshot = renderSnapshot;
        JSONArray expanded = new JSONArray();
        for (String[] pair : expandedSegments.values()) expanded.put(new JSONArray(java.util.Arrays.asList(pair)));
        final String expandedJson = expanded.toString();
        final int markerVisibility = showMarkers ? 1 : 0;
        chunkInFlight = true;
        // Rasterize to the actual displayed page width, not an image that
        // must be enlarged after rasterization. Before layout, the page
        // fills the display apart from the root's horizontal padding.
        int pageWidth = previewPages.getWidth();
        if (pageWidth <= 0) pageWidth = Math.max(1, getResources().getDisplayMetrics().widthPixels - dp(24));
        final float scale = pageWidth / PAGE_WIDTH_PT;
        final int chunkSize = scale > 3.0f ? 1 : CHUNK;

        renderer.execute(new Runnable() {
            @Override
            public void run() {
                // Do not spend time rasterising a queued, superseded request.
                if (generation != renderGeneration) {
                    return;
                }
                final String json = DocBridge.renderPagesWithSnapshot(text, snapshot, start, chunkSize, scale, caret, markerVisibility, expandedJson);
                main.post(new Runnable() {
                    @Override
                    public void run() {
                        if (generation == renderGeneration) {
                            chunkInFlight = false;
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
                showPreviewError(error);
                return;
            }

            totalPages = o.getInt("total");
            JSONArray pages = o.getJSONArray("pages");
            JSONArray warnings = o.optJSONArray("warnings");
            trace("applyChunk", "total", totalPages, "pages", pages.length());
            for (int i = 0; i < pages.length(); i++) {
                JSONObject p = pages.getJSONObject(i);
                int w = p.getInt("width");
                int h = p.getInt("height");
                double textTop = p.optDouble("textTop", -1);
                final JSONArray hits = p.getJSONArray("hits");
                byte[] png = Base64.decode(p.getString("png"), Base64.DEFAULT);
                Bitmap bmp = BitmapFactory.decodeByteArray(png, 0, png.length);
                if (bmp == null || bmp.getWidth() != w || bmp.getHeight() != h) {
                    throw new IllegalStateException("bad raster " + w + "x" + h);
                }
                int visible = 0;
                int[] row = new int[w];
                for (int yy = 0; yy < h; yy += 4) {
                    bmp.getPixels(row, 0, w, 0, yy, w, 1);
                    for (int px : row) { if ((px >>> 24) != 0) visible++; }
                }
                trace("pageRaster", "wh", w + "x" + h, "visiblePx~", visible * 4);

                TextView caption = new TextView(this);
                caption.setText("page " + (pagesLoaded + 1) + " / " + totalPages);
                caption.setTextColor(Color.parseColor("#6E7A87"));
                caption.setTextSize(10f);
                caption.setPadding(0, dp(PAGE_GAP_DP), 0, dp(2));
                previewPages.addView(caption);

                RenderedPage pageView = new RenderedPage(this, hits);
                renderedPages.add(pageView);
                // mathed's canvas, not "paper white": the page raster is
                // white text on transparent (THEME_PRELUDE sets fill:
                // white) with tinted figure slots, so it must sit on the
                // same dark canvas mathed_android's engine paints
                // (BG #14181D) or the text is invisible.
                pageView.setBackgroundColor(Color.parseColor("#14181D"));
                pageView.setImageBitmap(bmp);
                pageView.setClickable(true);
                // A tap on rendered text moves the source caret to the same
                // glyph. ScrollView still intercepts a drag as usual.
                pageView.setOnTouchListener(new View.OnTouchListener() {
                    private float x;
                    private float y;

                    @Override
                    public boolean onTouch(View view, MotionEvent event) {
                        if (event.getActionMasked() == MotionEvent.ACTION_UP) {
                            x = event.getX();
                            y = event.getY();
                            selectGlyph(view);
                        }
                        return true;
                    }

                    void selectGlyph(View view) {
                        if (view.getWidth() <= 0) return;
                        float scale = PAGE_WIDTH_PT / view.getWidth();
                        float px = x * scale;
                        float py = y * scale;
                        try {
                            for (int i = 0; i < hits.length(); i++) {
                                JSONArray h = hits.getJSONArray(i);
                                float left = (float) h.getDouble(0);
                                float right = (float) h.getDouble(2);
                                if (px >= Math.min(left, right) && px <= Math.max(left, right)
                                        && py >= h.getDouble(1) && py <= h.getDouble(3)) {
                                    int caret = Math.max(0, Math.min(h.getInt(4), editor.length()));
                                    int citation = citationAt(caret);
                                    if (citation > 0) { showCitations(citation); return; }
                                    if (toggleFoldAt(caret, true)) return;
                                    selectionAnchor = -1;
                                    if (targetsField != null) commitTargetPairs(targetsField, false);
                                    inspectorOpen = true;
                                    wholeDocumentEditing = false;
                                    editor.setSelection(caret);
                                    updateModifierPanel();
                                    focusSourceCaret();
                                    return;
                                }
                            }
                        } catch (Exception e) {
                            Log.w("emthin-pad", "could not select preview text", e);
                        }
                        if (hits.length() == 0) {
                            inspectorOpen = true;
                            wholeDocumentEditing = false;
                            inspectorSignature = "";
                            updateModifierPanel();
                            focusSourceCaret();
                        }
                    }
                });
                // Explicit EXACTLY height, not adjustViewBounds: on the
                // device (waydroid API 33) adjustViewBounds(true) +
                // wrap-content measured ~428px for a 1489x2105 raster
                // (~1/6 of the aspect-correct 2674), squeezing text to
                // sub-pixel = black screen. A LayoutParams pixel height
                // gives the child an EXACTLY spec, bypassing ImageView's
                // own bounds adjustment; FIT_CENTER does the (aspect-
                // preserving, full-bleed) fit into it.
                int pw = previewPages.getWidth();
                if (pw <= 0) {
                    // pre-layout fallback: full display width minus the
                    // root's 14px side margins (FIT_CENTER letterboxes a
                    // few px if slightly off; the exact value is cosmetic)
                    pw = Math.max(1, getResources().getDisplayMetrics().widthPixels - 28);
                }
                int ih = Math.max(1, Math.round(pw * (float) h / w));
                previewPages.addView(pageView, new LinearLayout.LayoutParams(
                        LinearLayout.LayoutParams.MATCH_PARENT, ih));
                if (!inspectorOpen && !(getCurrentFocus() instanceof EditText)) focusRenderedCaret();
                // Keep short documents visible while retaining the old
                // reading position in case undo restores a longer document.
                if (totalPages == 1
                        && restoreScrollY > 0 && !p.optBoolean("hasFigures", false)
                        && textTop >= 0) {
                    double bottom = -1;
                    for (int j = 0; j < hits.length(); j++) {
                        bottom = Math.max(bottom, hits.getJSONArray(j).getDouble(3));
                    }
                    int contentBottom = Math.round((float) bottom * pw / PAGE_WIDTH_PT) + dp(PAGE_GAP_DP + 20);
                    if (bottom >= 0 && restoreScrollY > contentBottom) {
                        visibleScrollY = Math.max(0, Math.round((float) textTop * pw / PAGE_WIDTH_PT));
                    }
                }
                trace("pageLayout", "raster", w + "x" + h, "pw", pw, "ih", ih,
                        "textTop", textTop);

                pagesLoaded++;
                // The first caption may not be on page 1 when a leading
                // figure fills the page; the renderer reports null for
                // pages before it, so take the first page that has one.
                if (scrollToFirstCaption && textTop >= 0) {
                    scrollToFirstCaption = false;
                    final ImageView firstPage = pageView;
                    final float captionTopPt = (float) textTop;
                    final int fallbackWidth = pw;
                    final int captionPage = pagesLoaded;
                    previewScroll.post(new Runnable() {
                        @Override
                        public void run() {
                            if (firstPage.getParent() == null) {
                                return; // a newer render replaced the pages
                            }
                            int width = firstPage.getWidth() > 0
                                    ? firstPage.getWidth() : fallbackWidth;
                            int offset = Math.round(captionTopPt * width / PAGE_WIDTH_PT);
                            int y = Math.max(0, firstPage.getTop() + offset - dp(4));
                            previewScroll.scrollTo(0, y);
                            trace("captionScroll", "page", captionPage,
                                    "topPt", captionTopPt, "px", offset, "scrollY", y);
                        }
                    });
                }
            }
            restorePreviewScroll();
            if (searchBar.getVisibility() == View.VISIBLE && searchCurrent >= 0) scrollSearchMatch();
            if (warnings != null && warnings.length() > 0) {
                StringBuilder message = new StringBuilder();
                for (int i = 0; i < warnings.length(); i++) {
                    if (i > 0) message.append('\n');
                    message.append(warnings.getString(i));
                }
                previewError.setText(message.toString());
                previewError.setVisibility(View.VISIBLE);
            } else {
                previewError.setVisibility(View.GONE);
            }
        } catch (Exception e) {
            showPreviewError("render: " + e.getMessage());
        }
    }

    /**
     * Drops the old page rasters before the rebuild. removeAllViews()
     * alone leaves the bitmaps for a later GC; on the pad's large pages
     * that is roughly 25 MB of Java heap held while the new render (and
     * its own base64 payload) is in flight.
     */
    private void releasePageBitmaps() {
        renderedPages.clear();
        int released = 0;
        for (int i = previewPages.getChildCount() - 1; i >= 0; i--) {
            View child = previewPages.getChildAt(i);
            if (!(child instanceof ImageView)) {
                continue;
            }
            ImageView page = (ImageView) child;
            Drawable drawable = page.getDrawable();
            page.setImageDrawable(null);
            if (drawable instanceof BitmapDrawable) {
                Bitmap bmp = ((BitmapDrawable) drawable).getBitmap();
                if (bmp != null && !bmp.isRecycled()) {
                    bmp.recycle();
                    released++;
                }
            }
        }
        previewPages.removeAllViews();
        trace("releasePages", "bitmaps", released);
    }

    /** Reapplies the pre-render offset once the rebuilt pages are in place. */
    private void restorePreviewScroll() {
        if (restoreScrollY < 0) {
            return;
        }
        final int target = restoreScrollY;
        final int visibleTarget = visibleScrollY >= 0 ? visibleScrollY : target;
        final int generation = renderGeneration;
        previewScroll.post(new Runnable() {
            @Override
            public void run() {
                // A touch or a newer render may have superseded this target
                // between the post and this runnable.
                if (generation != renderGeneration
                        || previewPages.getChildCount() == 0
                        || restoreScrollY != target) {
                    return;
                }
                previewScroll.scrollTo(0, visibleTarget);
                int actual = previewScroll.getScrollY();
                if (visibleTarget == target && actual == target) {
                    restoreScrollY = -1;
                } else {
                    // The rebuilt document is shorter than the retained
                    // position. Keep the desired offset: the next render
                    // (e.g. undo restoring the page count) should retry it,
                    // rather than anchoring on this clamped maximum.
                    restoreScrollY = target;
                }
                trace("restoreScroll", "gen", generation, "y", target, "actual", actual);
            }
        });
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
            trace("applyState", "setCaret", setCaret, "rev", o.optLong("revision"),
                    "textLen", text.length(), "canUndo", o.optBoolean("canUndo"));
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
            documentMarkers = o.optJSONArray("markers");
            if (documentMarkers == null) documentMarkers = new JSONArray();
            documentModifierLinks = o.optJSONArray("modifierLinks");
            if (documentModifierLinks == null) documentModifierLinks = new JSONArray();
            documentCitations = new JSONArray(DocBridge.citationIndex(text));
            documentSegments = o.optJSONArray("segments");
            if (documentSegments == null) documentSegments = new JSONArray();
            refreshSegments(o);
            updateMarkerVisibility();
            if (!editingModifier) updateSourceMarkerVisibility();
            updateModifierPanel();
            if (!editingModifier && modifierPanel.getVisibility() == View.VISIBLE) {
                scheduleModifierComparisons();
            }
            saveDocument();
            scheduleRender();
            if (searchBar != null && searchBar.getVisibility() == View.VISIBLE) updateSearch();
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
