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
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputConnectionWrapper;
import android.widget.HorizontalScrollView;
import android.widget.EditText;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;

import org.json.JSONArray;
import org.json.JSONObject;

import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
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
    private static final String PREF_LEGACY_KEEP = "legacyRecoveryKeep";
    private static final String PREF_DOCUMENT_URI = "document_uri";
    private static final String PREF_DOCUMENT_SNAPSHOT = "document_snapshot";
    private static final String PREF_DOCUMENT_HASH = "document_hash";
    private static final String PREF_DOCUMENT_SIZE = "document_size";
    private static final String PREF_DOCUMENT_MODIFIED = "document_modified";

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
    private StableAnchor selectionAnchorStable;
    /** Touch selection state; the long-press timeout may be pending. */
    private LinearLayout selectionBar;
    private RenderedPage touchSelectionPage;
    private int touchSelectionAnchor = -1;
    private float touchDownX, touchDownY;
    private boolean touchSelecting;
    /** Source offset of the virtual accessibility text block with focus. */
    private int accessibilityFocusOffset = -1;
    private final Runnable touchLongPress = new Runnable() {
        @Override public void run() {
            if (touchSelectionPage == null || !touchSelectionPage.loaded || pagesStale) return;
            int offset = caretOffsetAtPoint(touchSelectionPage, touchDownX, touchDownY);
            if (offset < 0) return;
            selectWordAt(offset);
            touchSelecting = true;
            touchSelectionPage.performHapticFeedback(
                    android.view.HapticFeedbackConstants.LONG_PRESS);
            showSelectionBar();
        }
    };
    private StableAnchor readingAnchorStable;
    private int readingAnchorFallback = -1;
    private boolean scrollToReadingAnchorAfterChunk;
    private int editorMarkerOffset = -1;
    private int editorTabOffset = -1;
    private int sourceCompletionAt = -1;
    private boolean modifierRefreshDeferred;
    private String modelText = "";
    private boolean pasting;
    private static final int OPEN_DOCUMENT = 41;
    private static final int SAVE_DOCUMENT = 42;
    private static final int EXPORT_RECOVERY = 43;
    private static final int SAVE_COPY_DOCUMENT = 44;
    private Uri documentUri;
    private boolean documentIsSnapshot = true;
    /** Content identity last written by an explicit Save/Open, not recovery. */
    private String savedFingerprint = "";
    /** True while the live model differs from the explicit file acknowledgement. */
    private boolean unsaved;
    /** Suppresses dirty marking while installing recovered/opened state. */
    private boolean restoringState;
    /** Provider identity remembered at Open or successful Save. */
    private ProviderIo.Stamp fileStamp;
    private PendingPicker pendingPicker;
    private FileOperation activeFile;
    private FileOperation owedFileSave;
    private FileOperation waitingFile;
    private AlertDialog fileCancelDialog;
    private final Runnable showFileCancel = new Runnable() {
        @Override public void run() { showFileCancelForActiveOperation(); }
    };
    private final PadSession session = new PadSession();
    /** Provider work is independent of view/render ownership. */
    private final ExecutorService fileWorker = Executors.newSingleThreadExecutor();
    /** The sole recovery/view writer; provider waits cannot block it. */
    private final ExecutorService recoveryWorker = Executors.newSingleThreadExecutor();
    private RecoveryStore recoveryStore;
    private RecoveryCodec.Metadata recoveryMetadata;
    private RecoveryCapture recoveryInFlight;
    private RecoveryCapture recoveryOwed;
    private long recoveryGeneration;
    private String durableFingerprint = "";
    private boolean recoveryReady;
    private boolean watcherInstalled;
    private boolean legacyPending;
    private boolean legacyKeep;
    private boolean recoveryViewInFlight;
    private boolean recoveryViewOwed;
    private long recoveryBurstStart;
    private long lastViewRequestAt;
    private Boolean bundleShowMarkers;
    private Boolean bundleInspectorOpen;
    private Runnable checkpointAction;
    private Runnable recoveryChoiceRetry;
    private byte[] recoveryExport;
    private final Runnable recoveryNow = new Runnable() {
        @Override public void run() { captureRecoveryNow(); }
    };
    private final Runnable viewNow = new Runnable() {
        @Override public void run() { captureViewNow(); }
    };
    /** One caption + {@link ImageView} per rendered page, stacked. */
    private LinearLayout previewPages;
    private LinearLayout previewBox;
    private ScrollView previewScroll;
    private HorizontalScrollView previewHScroll;
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
    /** Reflow changes the document height; restore the stable text anchor. */
    private boolean anchorNextRender;
    private int visibleScrollY = -1;
    /** Width at which a restored pixel offset was recorded, or 0 for legacy saves. */
    private int restoreScrollWidth;
    /** True while we mutate the EditText ourselves (suppresses sync). */
    private boolean applying;

    /**
     * One serial expensive-work worker for page layout/raster, inspector
     * comparisons and citation details. Each submission carries an immutable
     * {@link PadSession.Owner}; a newer edit or presentation supersedes it.
     */
    private final RenderController renderWork =
            new RenderController(Executors.newSingleThreadExecutor());
    private final Handler main = new Handler(Looper.getMainLooper());
    /** Identity of the page stream currently allowed to publish. */
    private PadSession.Owner renderOwner;
    /** A newer edit is waiting for the debounce timer. */
    private boolean renderPending;
    /** Pages currently stacked in the preview. */
    private int pagesLoaded;
    /** The document's full page count, from the latest chunk. */
    private int totalPages;
    /** One chunk in flight at a time (the renderer is single-threaded). */
    private boolean chunkInFlight;
    /** True until the first layout has located and loaded its reading page. */
    private boolean initialPagePending;
    private boolean initialLocatePending;
    /** Caption source offset from the retained-layout metadata. */
    private int captionStart = -1;
    /** Reading size in points; 17 is the pad default. */
    private float readingSizePt = 17f;
    /** Fit-width view zoom, 1..4. */
    private float viewZoom = 1f;
    /** Page bitmaps retained around the viewport, bounded by memory class. */
    private final java.util.ArrayList<RenderedPage> retainedPages = new java.util.ArrayList<>();
    private long retainedBitmapBytes;
    private long pageBitmapBudget;
    /** Immutable snapshot bytes captured on the UI thread for the worker. */
    private byte[] renderSnapshot;
    /** Worker-retained layout identity for the current page stream. */
    private String renderLayoutId;
    /** Layout identity whose pages are currently materialized. */
    private String publishedLayoutId;
    /** Old pages stay visible, but stale hit positions cannot edit. */
    private boolean pagesStale;
    /** Working-page RGBA cap; density is reduced to fit, geometry is not. */
    private long workingPagePixels = 4_000_000L;
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
        android.app.ActivityManager am =
                (android.app.ActivityManager) getSystemService(ACTIVITY_SERVICE);
        int memoryClass = am == null ? 128 : am.getMemoryClass();
        long budget = Math.min(48L << 20, Math.max(8L << 20, (memoryClass << 20) / 8));
        pageBitmapBudget = budget;
        workingPagePixels = Math.min(4_000_000L, budget / 16);
        readingSizePt = Math.max(8f, Math.min(32f, prefs.getFloat("readingSizePt", 17f)));
        viewZoom = Math.max(1f, Math.min(4f, prefs.getFloat("viewZoom", 1f)));
        showMarkers = prefs.getBoolean(PREF_SHOW_MARKERS, false);
        legacyKeep = prefs.getBoolean(PREF_LEGACY_KEEP, false);
        inspectorOpen = showMarkers;
        String savedUri = prefs.getString(PREF_DOCUMENT_URI, null);
        if (savedUri != null) documentUri = Uri.parse(savedUri);
        documentIsSnapshot = prefs.getBoolean(PREF_DOCUMENT_SNAPSHOT, true);
        String rememberedHash = prefs.getString(PREF_DOCUMENT_HASH, "");
        if (rememberedHash != null && !rememberedHash.isEmpty()) {
            fileStamp = new ProviderIo.Stamp(rememberedHash,
                    prefs.getLong(PREF_DOCUMENT_SIZE, 0),
                    prefs.getLong(PREF_DOCUMENT_MODIFIED, 0));
        }

        buildUi();
        previewBox.post(new Runnable() {
            @Override public void run() { applyViewZoomWidth(); }
        });
        // The editor exists so the view hierarchy is complete, but it must
        // not accept an edit that recovery is still loading.
        editor.setEnabled(false);
        status.setText("Restoring document…");
        recoveryStore = new RecoveryStore(getFilesDir());

        final int bundleCaret = savedInstanceState != null
                ? savedInstanceState.getInt(PREF_CARET, -1) : -1;
        final int bundleScroll = savedInstanceState != null
                ? savedInstanceState.getInt(PREF_SCROLL, -1) : -1;
        final int bundleScrollWidth = savedInstanceState != null
                ? savedInstanceState.getInt(PREF_SCROLL_WIDTH, 0) : 0;
        final String bundleDocument = savedInstanceState != null
                ? savedInstanceState.getString("recovery_document_id") : null;
        bundleShowMarkers = savedInstanceState != null && savedInstanceState.containsKey(PREF_SHOW_MARKERS)
                ? savedInstanceState.getBoolean(PREF_SHOW_MARKERS) : null;
        bundleInspectorOpen = savedInstanceState != null && savedInstanceState.containsKey("inspectorOpen")
                ? savedInstanceState.getBoolean("inspectorOpen") : null;
        final SharedPreferences preferences = prefs;
        recoveryWorker.execute(new Runnable() {
            @Override public void run() {
                final RecoveryStore.Load load = recoveryStore.load();
                final Map<String, Object> view = recoveryStore.readView();
                final Legacy legacy = readLegacy(preferences);
                main.post(new Runnable() {
                    @Override public void run() {
                        installInitialRecovery(load, view, legacy,
                                bundleDocument, bundleCaret, bundleScroll, bundleScrollWidth);
                    }
                });
            }
        });

        // The australVM Cranelift lane is a diagnostic, not normal startup
        // work. `device-lane.sh` launches with this exact extra so the logcat
        // proof still exists without a JIT compile on every editor open.
        if (getIntent().getBooleanExtra("emthin.jitSmoke", false)) {
            Log.i("emthin-pad", "cranelift roundtrip: " + DocBridge.jitRoundtrip(42));
        }
    }

    private void installEditorWatcher() {
        editor.addTextChangedListener(new TextWatcher() {
            @Override public void beforeTextChanged(CharSequence s, int st, int c, int a) {}

            @Override
            public void onTextChanged(CharSequence s, int st, int b, int c) {
                if (!applying) {
                    editorMarkerOffset = !pasting && c == 1 && s.charAt(st) == '#' ? st : -1;
                    editorTabOffset = b == 0 && c == 1 && s.charAt(st) == '\t' ? st : -1;
                }
            }

            @Override public void afterTextChanged(Editable s) { commitEditorText(s); }
        });
    }

    /** Commit a non-composing editor buffer as one native edit. */
    private void commitEditorText(Editable s) {
        if (applying || handle == 0 || isComposing(editor)) return;
        if (s.toString().equals(modelText) && editorMarkerOffset < 0 && editorTabOffset < 0) {
            savePosition();
            return;
        }
        if (editorTabOffset >= 0 && editorTabOffset < s.length()
                && s.charAt(editorTabOffset) == '\t') {
            int at = editorTabOffset;
            editorTabOffset = -1;
            applying = true;
            try {
                s.delete(at, at + 1);
                editor.setSelection(at);
            } finally {
                applying = false;
            }
            toggleMarkers();
        } else {
            editorTabOffset = -1;
        }
        if (editorMarkerOffset >= 0 && editorMarkerOffset < s.length()
                && s.charAt(editorMarkerOffset) == '#') {
            int at = editorMarkerOffset;
            editorMarkerOffset = -1;
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
        } else {
            editorMarkerOffset = -1;
        }
        if (s.toString().equals(modelText)) {
            savePosition();
            return;
        }
        int caret = editor.getSelectionStart();
        String state = DocBridge.sync(handle, s.toString(), caret);
        applyState(state, false);
    }

    /**
     * A native text field that keeps IME candidates in the field until the
     * input connection finishes composition. The text watchers deliberately
     * ignore candidate updates, and the fields expose a synchronous flush for
     * Save/New/Open and inspector teardown.
     */
    private abstract class CompositionEditText extends EditText {
        private boolean composing;
        private InputConnection connection;

        CompositionEditText(android.content.Context context) { super(context); }

        boolean isComposing() {
            Editable text = getText();
            return composing || BaseInputConnection.getComposingSpanStart(text) >= 0
                    || BaseInputConnection.getComposingSpanEnd(text) >= 0;
        }

        @Override public InputConnection onCreateInputConnection(EditorInfo out) {
            final InputConnection base = super.onCreateInputConnection(out);
            if (base == null) return null;
            connection = new InputConnectionWrapper(base, true) {
                @Override public boolean setComposingText(CharSequence text, int cursor) {
                    composing = true;
                    return super.setComposingText(text, cursor);
                }
                @Override public boolean setComposingRegion(int start, int end) {
                    composing = true;
                    return super.setComposingRegion(start, end);
                }
                @Override public boolean finishComposingText() {
                    boolean was = composing;
                    boolean result;
                    try { result = super.finishComposingText(); }
                    finally {
                        if (was) {
                            composing = false;
                            BaseInputConnection.removeComposingSpans(getText());
                            compositionCommitted();
                        }
                    }
                    return result;
                }
                @Override public boolean commitText(CharSequence text, int cursor) {
                    boolean was = composing;
                    boolean result;
                    try { result = super.commitText(text, cursor); }
                    finally {
                        if (was) {
                            composing = false;
                            BaseInputConnection.removeComposingSpans(getText());
                            compositionCommitted();
                        }
                    }
                    return result;
                }
            };
            return connection;
        }

        @Override protected void onFocusChanged(boolean focused, int direction,
                                                android.graphics.Rect previous) {
            if (!focused) finishComposition();
            super.onFocusChanged(focused, direction, previous);
        }

        @Override public boolean onTextContextMenuItem(int id) {
            boolean wasPasting = pasting;
            if (id == android.R.id.paste) pasting = true;
            try { return super.onTextContextMenuItem(id); }
            finally { pasting = wasPasting; }
        }

        void finishComposition() {
            boolean active = composing || BaseInputConnection.getComposingSpanStart(getText()) >= 0
                    || BaseInputConnection.getComposingSpanEnd(getText()) >= 0;
            if (!active) return;
            if (connection != null) connection.finishComposingText();
            if (composing || BaseInputConnection.getComposingSpanStart(getText()) >= 0
                    || BaseInputConnection.getComposingSpanEnd(getText()) >= 0) {
                composing = false;
                BaseInputConnection.removeComposingSpans(getText());
                compositionCommitted();
            }
        }

        protected abstract void compositionCommitted();
    }

    private static final class Legacy {
        String text;
        String snapshot;
        int caret = -1;
        int scroll = -1;
        int scrollWidth;
    }

    private static final class RecoveryCapture {
        final byte[] snapshot;
        final RecoveryCodec.Metadata metadata;

        RecoveryCapture(byte[] snapshot, RecoveryCodec.Metadata metadata) {
            this.snapshot = snapshot;
            this.metadata = metadata;
        }
    }

    /** A picker launched by this document incarnation. */
    private static final class PendingPicker {
        final int requestCode;
        final PadSession.Owner owner;
        final boolean copy;

        PendingPicker(int requestCode, PadSession.Owner owner, boolean copy) {
            this.requestCode = requestCode;
            this.owner = owner;
            this.copy = copy;
        }
    }

    /** One active provider read/write; a newer same-URI save may wait. */
    private static final class FileOperation {
        final String kind;
        final PadSession.Owner owner;
        final Uri uri;
        final byte[] bytes;
        final boolean copy;
        final boolean replaceConfirmed;
        final boolean recheckBeforeReplace;
        final boolean activeDocument;
        ProviderIo.Stamp baseline;
        final String originalSnapshot;
        volatile boolean cancelled;
        volatile java.io.Closeable stream;
        java.util.concurrent.Future<?> future;

        FileOperation(String kind, PadSession.Owner owner, Uri uri, byte[] bytes,
                      boolean copy, boolean replaceConfirmed, boolean recheckBeforeReplace,
                      boolean activeDocument, ProviderIo.Stamp baseline, String originalSnapshot) {
            this.kind = kind;
            this.owner = owner;
            this.uri = uri;
            this.bytes = bytes;
            this.copy = copy;
            this.replaceConfirmed = replaceConfirmed;
            this.recheckBeforeReplace = recheckBeforeReplace;
            this.activeDocument = activeDocument;
            this.baseline = baseline;
            this.originalSnapshot = originalSnapshot;
        }
    }

    private static Legacy readLegacy(SharedPreferences prefs) {
        Legacy legacy = new Legacy();
        legacy.text = prefs.getString(PREF_DOC, null);
        legacy.snapshot = prefs.getString(PREF_SNAPSHOT, null);
        legacy.caret = prefs.getInt(PREF_CARET, -1);
        legacy.scroll = prefs.getInt(PREF_SCROLL, -1);
        legacy.scrollWidth = prefs.getInt(PREF_SCROLL_WIDTH, 0);
        return legacy;
    }

    private void installInitialRecovery(RecoveryStore.Load load, Map<String, Object> view,
                                        Legacy legacy, String bundleDocument, int bundleCaret,
                                        int bundleScroll, int bundleScrollWidth) {
        if (load.best != null) {
            long restored = DocBridge.openSnapshotBytes(load.best.snapshot);
            if (restored == 0 && load.older != null) {
                // The newest structurally valid slot failed to import. Keep
                // it as evidence and fall back to the older valid slot.
                load.bad.add(new RecoveryStore.BadSlot(0, "snapshot import failed", false,
                        load.best.raw));
                load.best = load.older;
                load.older = null;
                restored = DocBridge.openSnapshotBytes(load.best.snapshot);
            }
            if (restored != 0) {
                installRecovered(restored, load.best.metadata, view,
                        bundleDocument, bundleCaret, bundleScroll, bundleScrollWidth);
                if (!load.bad.isEmpty()) {
                    status.setText("Recovered an earlier checkpoint · recovery evidence retained"
                            + saveStateLabel());
                }
                removeLegacy(prefs());
                return;
            }
            load.bad.add(new RecoveryStore.BadSlot(0, "snapshot import failed", false,
                    load.best.raw));
            load.best = null;
        }

        if (legacy.snapshot != null && legacy.text != null) {
            long restored = DocBridge.createFromSnapshot(legacy.snapshot, legacy.text);
            if (restored != 0) {
                legacyPending = true;
                installRecovered(restored, legacyMetadata(legacy.text), view,
                        bundleDocument, bundleCaret, bundleScroll, bundleScrollWidth);
                return;
            }
            offerTextOnlyRecovery(legacy, load.bad);
            return;
        }
        if (legacy.text != null) {
            legacyPending = true;
            RecoveryCodec.Metadata metadata = legacyMetadata(legacy.text);
            long restored = DocBridge.create(legacy.text);
            installRecovered(restored, metadata, view,
                    bundleDocument, bundleCaret, bundleScroll, bundleScrollWidth);
            return;
        }
        if (!load.bad.isEmpty() || !load.errors.isEmpty()) {
            showRecoveryChoice(load, legacy);
            return;
        }
        installFreshStarter();
    }

    private RecoveryCodec.Metadata legacyMetadata(String text) {
        RecoveryCodec.Metadata metadata = new RecoveryCodec.Metadata();
        metadata.documentId = "legacy-" + java.util.UUID.randomUUID();
        metadata.displayName = "Recovered document";
        metadata.caret = editor == null ? 0 : Math.max(0, editor.getSelectionStart());
        metadata.selectionEnd = metadata.caret;
        return metadata;
    }

    private void installRecovered(long restored, RecoveryCodec.Metadata metadata,
                                  Map<String, Object> view, String bundleDocument,
                                  int bundleCaret, int bundleScroll, int bundleScrollWidth) {
        handle = restored;
        recoveryMetadata = metadata;
        recoveryGeneration = Math.max(0, metadata.generation);
        durableFingerprint = metadata.logicalFingerprint == null ? "" : metadata.logicalFingerprint;
        session.adoptDocument(metadata.documentId);
        Uri recoveredUri = metadata.uri == null || metadata.uri.isEmpty() ? null : Uri.parse(metadata.uri);
        if (documentUri == null || !documentUri.equals(recoveredUri)) fileStamp = null;
        documentUri = recoveredUri;
        documentIsSnapshot = metadata.fileSnapshot;
        String recoveredSaved = metadata.savedFingerprint == null ? "" : metadata.savedFingerprint;
        boolean explicitMatch = documentUri != null && fileStamp != null && fileStamp.known()
                && !recoveredSaved.isEmpty()
                && recoveredSaved.equals(metadata.logicalFingerprint);
        savedFingerprint = explicitMatch ? recoveredSaved : "";
        unsaved = !explicitMatch;
        recoveryReady = true;
        editor.setEnabled(true);
        if (!watcherInstalled) {
            watcherInstalled = true;
            installEditorWatcher();
        }
        restoringState = true;
        applyState(DocBridge.state(handle), true);
        restoringState = false;
        firstRender = false;

        boolean useBundle = bundleDocument == null || bundleDocument.equals(metadata.documentId);
        if (!metadata.documentId.equals(String.valueOf(view.get("documentId")))
                || (!metadata.logicalFingerprint.isEmpty()
                    && !metadata.logicalFingerprint.equals(String.valueOf(view.get("logicalFingerprint")))))
            view = java.util.Collections.emptyMap();
        int caret = useBundle && bundleCaret >= 0 ? bundleCaret : intValue(view, "caret", metadata.caret);
        int selection = useBundle && bundleCaret >= 0
                ? bundleCaret : intValue(view, "selectionEnd", metadata.selectionEnd);
        showMarkers = bundleShowMarkers != null ? bundleShowMarkers
                : boolValue(view, "showMarkers", metadata.showMarkers);
        inspectorOpen = bundleInspectorOpen != null ? bundleInspectorOpen
                : boolValue(view, "inspectorOpen", metadata.inspectorOpen);
        expandedSegments.clear();
        Object folds = view.get("folds");
        if (folds instanceof java.util.List) {
            for (Object item : (java.util.List<?>) folds) {
                if (item instanceof java.util.List && ((java.util.List<?>) item).size() == 2) {
                    java.util.List<?> pair = (java.util.List<?>) item;
                    expandedSegments.put(String.valueOf(pair.get(0)),
                            new String[] {String.valueOf(pair.get(0)), String.valueOf(pair.get(1))});
                }
            }
        } else {
            for (java.util.List<String> pair : metadata.folds) {
                expandedSegments.put(pair.get(0), new String[] {pair.get(0), pair.get(1)});
            }
        }
        caret = Math.max(0, Math.min(editor.length(), caret));
        selection = Math.max(0, Math.min(editor.length(), selection));
        editor.setSelection(Math.min(caret, selection), Math.max(caret, selection));
        int scroll = useBundle && bundleScroll >= 0 ? bundleScroll : intValue(view, "scrollY", metadata.scrollY);
        int scrollWidth = useBundle && bundleScrollWidth > 0
                ? bundleScrollWidth : intValue(view, "scrollWidth", metadata.scrollWidth);
        if (scroll >= 0) {
            restoreScrollY = scroll;
            restoreScrollWidth = scrollWidth;
        }
        String readingEncoded = stringValue(view, "readingAnchorEncoded",
                metadata.readingAnchorEncoded);
        int readingSide = intValue(view, "readingAnchorSide", metadata.readingAnchorSide);
        int readingFallback = intValue(view, "readingFallback", metadata.readingFallback);
        readingAnchorFallback = readingFallback;
        readingAnchorStable = readingEncoded == null || readingEncoded.isEmpty()
                ? null
                : new StableAnchor(metadata.documentId, readingEncoded, readingSide, readingFallback);
        scrollToReadingAnchorAfterChunk = readingAnchorStable != null;
        updateMarkerVisibility();
        if (showMarkers && markersButton != null) markersButton.setText("Tab Hide markers");
        if (inspectorOpen) updateModifierPanel();
        scheduleRender();
        requestViewWrite(true);
        if (legacyPending) captureRecoveryNow();
    }

    private void installFreshStarter() {
        if (handle != 0) DocBridge.free(handle);
        handle = DocBridge.create(STARTER);
        session.beginDocument();
        recoveryGeneration = 0;
        durableFingerprint = "";
        legacyPending = false;
        recoveryReady = true;
        editor.setEnabled(true);
        if (!watcherInstalled) {
            watcherInstalled = true;
            installEditorWatcher();
        }
        firstRender = true;
        savedFingerprint = "";
        unsaved = true;
        fileStamp = null;
        restoringState = true;
        applyState(DocBridge.state(handle), true);
        restoringState = false;
        status.setText("New document · unsaved");
        captureRecoveryNow();
    }

    private void offerTextOnlyRecovery(Legacy legacy, java.util.List<RecoveryStore.BadSlot> bad) {
        editor.setEnabled(false);
        byte[] snapshot = decodeLegacySnapshot(legacy.snapshot);
        recoveryExport = snapshot;
        recoveryChoiceRetry = new Runnable() {
            @Override public void run() { offerTextOnlyRecovery(legacy, bad); }
        };
        AlertDialog dialog = new AlertDialog.Builder(this, android.R.style.Theme_Material_Dialog_Alert)
                .setTitle("Snapshot could not be restored")
                .setMessage("The saved text can be opened, but its marks cannot. The original snapshot is retained for export.")
                .setCancelable(false)
                .setPositiveButton("Recover text without marks", new DialogInterface.OnClickListener() {
                    @Override public void onClick(DialogInterface dialog, int which) {
                        legacyKeep = true;
                        legacyPending = true;
                        prefs().edit().putBoolean(PREF_LEGACY_KEEP, true).apply();
                        long restored = DocBridge.create(legacy.text);
                        installRecovered(restored, legacyMetadata(legacy.text), java.util.Collections.emptyMap(),
                                null, -1, -1, 0);
                        status.setText("Recovered text without marks · original snapshot retained · unsaved");
                    }
                })
                .setNeutralButton("Export snapshot", new DialogInterface.OnClickListener() {
                    @Override public void onClick(DialogInterface dialog, int which) {
                        exportRecoveryData();
                    }
                })
                .setNegativeButton("Start new", new DialogInterface.OnClickListener() {
                    @Override public void onClick(DialogInterface dialog, int which) {
                        if (!quarantineLegacy(snapshot)) return;
                        legacyKeep = false;
                        prefs().edit().putBoolean(PREF_LEGACY_KEEP, false).apply();
                        installFreshStarter();
                    }
                }).create();
        dialog.setOnDismissListener(new DialogInterface.OnDismissListener() {
            @Override public void onDismiss(DialogInterface ignored) {
                if (handle == 0 && recoveryChoiceRetry != null) main.post(recoveryChoiceRetry);
            }
        });
        dialog.show();
    }

    private void showRecoveryChoice(RecoveryStore.Load load, Legacy legacy) {
        editor.setEnabled(false);
        status.setText("Recovery needs a choice");
        java.io.ByteArrayOutputStream export = new java.io.ByteArrayOutputStream();
        for (RecoveryStore.BadSlot bad : load.bad) {
            if (bad.raw == null) continue;
            export.write(bad.raw, 0, bad.raw.length);
        }
        byte[] legacyBytes = decodeLegacySnapshot(legacy.snapshot);
        if (legacyBytes != null) export.write(legacyBytes, 0, legacyBytes.length);
        recoveryExport = export.size() == 0 ? null : export.toByteArray();
        recoveryChoiceRetry = new Runnable() {
            @Override public void run() { showRecoveryChoice(load, legacy); }
        };
        AlertDialog dialog = new AlertDialog.Builder(this, android.R.style.Theme_Material_Dialog_Alert)
                .setTitle("Recovery needs a choice")
                .setMessage("Unreadable recovery data is retained. Export it before starting new content.")
                .setCancelable(false)
                .setPositiveButton("Export recovery data", new DialogInterface.OnClickListener() {
                    @Override public void onClick(DialogInterface dialog, int which) {
                        exportRecoveryData();
                    }
                })
                .setNeutralButton("Open another document", new DialogInterface.OnClickListener() {
                    @Override public void onClick(DialogInterface dialog, int which) {
                        installFreshStarter();
                        chooseDocument();
                    }
                })
                .setNegativeButton("Start new", new DialogInterface.OnClickListener() {
                    @Override public void onClick(DialogInterface dialog, int which) {
                        installFreshStarter();
                    }
                }).create();
        dialog.setOnDismissListener(new DialogInterface.OnDismissListener() {
            @Override public void onDismiss(DialogInterface ignored) {
                if (handle == 0 && recoveryChoiceRetry != null) main.post(recoveryChoiceRetry);
            }
        });
        dialog.show();
    }

    private boolean quarantineLegacy(byte[] bytes) {
        if (bytes == null) return true;
        try {
            recoveryStore.quarantine(0, bytes);
            return true;
        } catch (java.io.IOException e) {
            fileMessage("Could not preserve the old snapshot; nothing was replaced.");
            return false;
        }
    }

    private static byte[] decodeLegacySnapshot(String encoded) {
        if (encoded == null) return null;
        try {
            return Base64.decode(encoded, Base64.DEFAULT);
        } catch (RuntimeException e) {
            return null;
        }
    }

    private void removeLegacy(SharedPreferences prefs) {
        SharedPreferences.Editor editor = prefs.edit().remove(PREF_DOC);
        if (!legacyKeep) {
            editor.remove(PREF_SNAPSHOT).remove(PREF_LEGACY_KEEP);
        }
        editor.apply();
    }

    private SharedPreferences prefs() {
        return getSharedPreferences(PREFS, MODE_PRIVATE);
    }

    private static int intValue(Map<String, Object> map, String key, int fallback) {
        Object value = map.get(key);
        return value instanceof Number ? ((Number) value).intValue() : fallback;
    }

    private static String stringValue(Map<String, Object> map, String key, String fallback) {
        Object value = map.get(key);
        return value instanceof String ? (String) value : fallback;
    }

    private static boolean boolValue(Map<String, Object> map, String key, boolean fallback) {
        Object value = map.get(key);
        return value instanceof Boolean ? (Boolean) value : fallback;
    }

    @Override
    protected void onSaveInstanceState(Bundle outState) {
        super.onSaveInstanceState(outState);
        if (editor != null) {
            outState.putString("recovery_document_id", session.documentId());
            outState.putBoolean(PREF_SHOW_MARKERS, showMarkers);
            outState.putBoolean("inspectorOpen", inspectorOpen);
            outState.putInt(PREF_CARET, Math.max(0, editor.getSelectionStart()));
            outState.putInt(PREF_SCROLL, readingY());
            outState.putInt(PREF_SCROLL_WIDTH, readingWidth());
        }
    }

    @Override
    protected void onPause() {
        savePosition();
        // Capture on the UI thread now, then let the owned worker finish the
        // encode/fsync even if this Activity is destroyed.
        captureRecoveryNow();
        captureViewNow();
        super.onPause();
    }

    /** Captures on the UI thread, then encodes/writes on the recovery worker. */
    private void scheduleRecovery(boolean immediate) {
        if (!recoveryReady || handle == 0 || recoveryStore == null) return;
        long now = android.os.SystemClock.uptimeMillis();
        if (now - recoveryBurstStart > 2000) recoveryBurstStart = now;
        main.removeCallbacks(recoveryNow);
        if (immediate || now - recoveryBurstStart >= 2000) {
            main.post(recoveryNow);
        } else {
            main.postDelayed(recoveryNow, 250);
        }
    }

    private void saveDocument() {
        scheduleRecovery(false);
        requestViewWrite(false);
    }

    /**
     * Writes small view metadata for the next launch. During a rebuild the
     * pages are briefly removed and the ScrollView clamps to 0, so fall back
     * to the retained target until the pages are back.
     */
    private void savePosition() {
        if (editor == null || previewScroll == null) return;
        prefs().edit()
                .putInt(PREF_CARET, Math.max(0, editor.getSelectionStart()))
                .putInt(PREF_SCROLL, readingY())
                .putInt(PREF_SCROLL_WIDTH, readingWidth())
                .apply();
    }

    private void captureRecoveryNow() {
        main.removeCallbacks(recoveryNow);
        if (!recoveryReady || handle == 0 || recoveryStore == null) return;
        byte[] snapshot = DocBridge.snapshotBytes(handle);
        if (snapshot == null || snapshot.length > RecoveryCodec.MAX_SNAPSHOT) {
            status.setText("Recovery cannot be updated: document exceeds 32 MiB");
            failCheckpoint();
            return;
        }
        RecoveryCodec.Metadata metadata = viewMetadata();
        metadata.generation = ++recoveryGeneration;
        RecoveryCapture capture = new RecoveryCapture(snapshot, metadata);
        if (recoveryInFlight != null) {
            recoveryOwed = capture;
            return;
        }
        submitRecovery(capture);
    }

    private void submitRecovery(final RecoveryCapture capture) {
        recoveryInFlight = capture;
        recoveryWorker.execute(new Runnable() {
            @Override public void run() {
                String error = null;
                String fingerprint = DocBridge.logicalFingerprint(capture.snapshot);
                try {
                    if (fingerprint == null || fingerprint.isEmpty()) {
                        throw new java.io.IOException("snapshot is not importable");
                    }
                    capture.metadata.logicalFingerprint = fingerprint;
                    recoveryStore.write(capture.metadata, capture.snapshot);
                } catch (Exception e) {
                    error = e.getMessage() == null ? e.toString() : e.getMessage();
                }
                final String failure = error;
                main.post(new Runnable() {
                    @Override public void run() { recoveryAcknowledged(capture, failure); }
                });
            }
        });
    }

    private void recoveryAcknowledged(RecoveryCapture capture, String failure) {
        recoveryInFlight = null;
        boolean gone = isDestroyed();
        if (failure != null) {
            recoveryGeneration = Math.max(0, capture.metadata.generation - 1);
            if (recoveryOwed != null) {
                RecoveryCapture next = recoveryOwed;
                recoveryOwed = null;
                submitRecovery(next);
                return;
            }
            if (!gone) {
                status.setText("Recovery failed: " + failure);
                if (legacyPending) fileMessage("Recovery could not be updated; old data is retained.");
                failCheckpoint();
            }
            return;
        }
        recoveryMetadata = capture.metadata;
        durableFingerprint = capture.metadata.logicalFingerprint;
        recoveryGeneration = capture.metadata.generation;
        legacyPending = false;
        removeLegacy(prefs());
        if (gone) return;
        if (checkpointAction == null && recoveryOwed == null)
            status.setText("Recovered · durable" + saveStateLabel());
        if (recoveryOwed != null) {
            RecoveryCapture next = recoveryOwed;
            recoveryOwed = null;
            submitRecovery(next);
            return;
        }
        if (checkpointAction != null) {
            Runnable action = checkpointAction;
            checkpointAction = null;
            action.run();
        }
    }

    /**
     * Make the current content durable before New/Open replaces it. An
     * already-captured write is allowed to finish; the newest owed capture
     * must be acknowledged too. Native application keeps running if Java
     * interruption cannot stop a Typst call.
     */
    private void checkpointThen(Runnable action) {
        if (handle == 0 || !recoveryReady) { action.run(); return; }
        checkpointAction = action;
        captureRecoveryNow();
    }

    private void failCheckpoint() {
        final Runnable action = checkpointAction;
        if (action == null) return;
        new AlertDialog.Builder(this, android.R.style.Theme_Material_Dialog_Alert)
                .setTitle("Recovery could not be saved")
                .setMessage("The current document is still open. Retry, or discard this checkpoint and continue.")
                .setPositiveButton("Retry", new DialogInterface.OnClickListener() {
                    @Override public void onClick(DialogInterface dialog, int which) {
                        captureRecoveryNow();
                    }
                })
                .setNegativeButton("Discard checkpoint", new DialogInterface.OnClickListener() {
                    @Override public void onClick(DialogInterface dialog, int which) {
                        checkpointAction = null;
                        action.run();
                    }
                }).show();
    }

    private RecoveryCodec.Metadata viewMetadata() {
        rememberReadingAnchor();
        RecoveryCodec.Metadata metadata = new RecoveryCodec.Metadata();
        metadata.documentId = session.documentId();
        metadata.logicalFingerprint = durableFingerprint;
        metadata.displayName = documentUri == null ? "Recovered document" : documentUri.getLastPathSegment();
        metadata.uri = documentUri == null ? "" : documentUri.toString();
        metadata.fileSnapshot = documentIsSnapshot;
        metadata.caret = editor == null ? 0 : Math.max(0, editor.getSelectionStart());
        metadata.selectionEnd = editor == null ? metadata.caret : Math.max(0, editor.getSelectionEnd());
        metadata.scrollY = readingY();
        metadata.scrollWidth = readingWidth();
        metadata.readingAnchor = readingAnchorFallback;
        metadata.readingAnchorEncoded = readingAnchorStable == null ? "" : readingAnchorStable.encoded;
        metadata.readingAnchorSide = readingAnchorStable == null ? -1 : readingAnchorStable.side;
        metadata.readingFallback = readingAnchorFallback;
        metadata.showMarkers = showMarkers;
        metadata.inspectorOpen = inspectorOpen;
        metadata.savedFingerprint = savedFingerprint;
        if (searchBar != null && searchBar.getVisibility() == View.VISIBLE && searchField != null) {
            metadata.query = searchField.getText().toString();
        }
        for (String[] pair : expandedSegments.values()) {
            metadata.folds.add(java.util.Arrays.asList(pair));
        }
        return metadata;
    }

    private void requestViewWrite(boolean immediate) {
        if (!recoveryReady || handle == 0 || recoveryStore == null) return;
        long now = android.os.SystemClock.uptimeMillis();
        if (now - lastViewRequestAt > 2000) lastViewRequestAt = now;
        main.removeCallbacks(viewNow);
        if (immediate || now - lastViewRequestAt >= 2000) main.post(viewNow);
        else main.postDelayed(viewNow, 250);
    }

    private void captureViewNow() {
        main.removeCallbacks(viewNow);
        if (!recoveryReady || handle == 0 || recoveryStore == null || isDestroyed()) return;
        if (recoveryViewInFlight) { recoveryViewOwed = true; return; }
        recoveryViewInFlight = true;
        final Map<String, Object> view = viewMetadata().toMap();
        recoveryWorker.execute(new Runnable() {
            @Override public void run() {
                try { recoveryStore.writeView(view); }
                catch (java.io.IOException ignored) { }
                main.post(new Runnable() {
                    @Override public void run() {
                        recoveryViewInFlight = false;
                        if (isDestroyed()) return;
                        if (recoveryViewOwed) {
                            recoveryViewOwed = false;
                            captureViewNow();
                        }
                    }
                });
            }
        });
    }

    private void exportRecoveryData() {
        if (recoveryExport == null || recoveryExport.length == 0) {
            fileMessage("No recovery bytes to export.");
            return;
        }
        Intent intent = new Intent(Intent.ACTION_CREATE_DOCUMENT);
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        intent.setType("application/octet-stream");
        intent.putExtra(Intent.EXTRA_TITLE, "emthin-recovery.bin");
        startActivityForResult(intent, EXPORT_RECOVERY);
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
    public void onTrimMemory(int level) {
        super.onTrimMemory(level);
        trimPageBitmaps();
    }

    @Override
    public void onLowMemory() {
        super.onLowMemory();
        trimPageBitmaps();
    }

    @Override
    protected void onDestroy() {
        pendingPicker = null;
        if (activeFile != null) activeFile.cancelled = true;
        if (owedFileSave != null) owedFileSave.cancelled = true;
        if (waitingFile != null) waitingFile.cancelled = true;
        dismissFileCancel();
        // Invalidate first: a result already posted to `main` must not touch
        // this view while the native handle is being freed.
        session.dispose();
        renderWork.close();
        main.removeCallbacks(renderNow);
        main.removeCallbacks(updateModifierComparisons);
        main.removeCallbacks(recoveryNow);
        main.removeCallbacks(viewNow);
        fileWorker.shutdown();
        // Already-captured bytes may finish; they never touch this view.
        recoveryWorker.shutdown();
        releasePageBitmaps();
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
        bar.addView(shortcut("^⇧S Copy", new Runnable() {
            @Override public void run() { saveCopyToDocument(); }
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
        bar.addView(shortcut("A−", new Runnable() {
            @Override public void run() { changeReadingSize(-1f); }
        }));
        bar.addView(shortcut("A+", new Runnable() {
            @Override public void run() { changeReadingSize(1f); }
        }));
        bar.addView(shortcut("Zoom−", new Runnable() {
            @Override public void run() { changeViewZoom(-0.5f); }
        }));
        bar.addView(shortcut("Zoom+", new Runnable() {
            @Override public void run() { changeViewZoom(0.5f); }
        }));
        bar.addView(close);
        HorizontalScrollView shortcuts = new HorizontalScrollView(this);
        shortcuts.setHorizontalScrollBarEnabled(false);
        shortcuts.addView(bar);
        root.addView(shortcuts, new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT));
        selectionBar = new LinearLayout(this);
        selectionBar.setOrientation(LinearLayout.HORIZONTAL);
        selectionBar.setBackgroundColor(Color.parseColor("#1A2229"));
        selectionBar.setVisibility(View.GONE);
        selectionBar.addView(shortcut("Copy", new Runnable() {
            @Override public void run() { copyRenderedSelection(); hideSelectionBar(); }
        }));
        selectionBar.addView(shortcut("Cut", new Runnable() {
            @Override public void run() {
                copyRenderedSelection();
                deleteRenderedSelection();
                hideSelectionBar();
            }
        }));
        selectionBar.addView(shortcut("Paste", new Runnable() {
            @Override public void run() { pasteRenderedClipboard(); hideSelectionBar(); }
        }));
        selectionBar.addView(shortcut("Select all", new Runnable() {
            @Override public void run() {
                setSelectionAnchor(0);
                setDocumentCaret(editor.length());
                showSelectionBar();
            }
        }));
        root.addView(selectionBar, new LinearLayout.LayoutParams(
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

        previewBox = new LinearLayout(this);
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
        previewHScroll = new HorizontalScrollView(this);
        previewHScroll.setFillViewport(false);
        previewHScroll.setHorizontalScrollBarEnabled(false);
        previewHScroll.addView(previewBox, new HorizontalScrollView.LayoutParams(
                HorizontalScrollView.LayoutParams.WRAP_CONTENT,
                HorizontalScrollView.LayoutParams.WRAP_CONTENT));
        previewScroll.addView(previewHScroll, new ScrollView.LayoutParams(
                ScrollView.LayoutParams.MATCH_PARENT, ScrollView.LayoutParams.WRAP_CONTENT));
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
                ensureVisiblePages();
                trimPageBitmaps();
                rememberReadingAnchor();
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

    /** Viewport width at zoom 1: the point-to-pixel basis for every page. */
    private int basePageWidth() {
        int width = previewHScroll == null ? 0 : previewHScroll.getWidth();
        if (width <= 0) {
            width = Math.max(1, getResources().getDisplayMetrics().widthPixels - dp(24));
        }
        return width;
    }

    private void applyViewZoomWidth() {
        if (previewBox == null) return;
        int width = Math.max(1,
                Math.round(basePageWidth() * viewZoom));
        android.view.ViewGroup.LayoutParams params = previewBox.getLayoutParams();
        if (params == null) {
            params = new android.widget.FrameLayout.LayoutParams(
                    width, android.view.ViewGroup.LayoutParams.WRAP_CONTENT);
        } else {
            params.width = width;
        }
        previewBox.setLayoutParams(params);
        previewBox.requestLayout();
    }

    private void changeReadingSize(float delta) {
        float next = Math.max(8f, Math.min(32f, readingSizePt + delta));
        if (next == readingSizePt) return;
        readingSizePt = next;
        getSharedPreferences(PREFS, MODE_PRIVATE).edit()
                .putFloat("readingSizePt", readingSizePt).apply();
        rememberReadingAnchor();
        anchorNextRender = true;
        firstRender = false;
        scheduleRender();
        trace("readingSize", "pt", readingSizePt);
    }

    private void changeViewZoom(float delta) {
        float next = Math.max(1f, Math.min(4f, viewZoom + delta));
        if (next == viewZoom) return;
        int scaledY = Math.round(Math.max(0, readingY()) * next / viewZoom);
        viewZoom = next;
        getSharedPreferences(PREFS, MODE_PRIVATE).edit()
                .putFloat("viewZoom", viewZoom).apply();
        restoreScrollY = scaledY;
        restoreScrollWidth = 0;
        visibleScrollY = -1;
        applyViewZoomWidth();
        firstRender = false;
        scheduleRender();
        trace("viewZoom", "zoom", viewZoom, "baseWidth", basePageWidth());
    }

    private void fileMessage(String text) {
        android.widget.Toast.makeText(this, text, android.widget.Toast.LENGTH_LONG).show();
    }

    private void chooseDocument() {
        if (handle == 0) return;
        if (pendingPicker != null) {
            fileMessage("Finish the file picker already open.");
            return;
        }
        finishActiveComposition();
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        intent.setType("*/*");
        intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
        launchPicker(intent, OPEN_DOCUMENT,
                new PendingPicker(OPEN_DOCUMENT, session.capture(PadSession.OPEN, "picker"), false));
    }

    private void saveToDocument() {
        if (handle == 0) return;
        if (pendingPicker != null) {
            fileMessage("Finish the file picker already open.");
            return;
        }
        if (documentUri == null) {
            launchSavePicker(false);
        } else {
            beginSave(documentUri, false, false, false);
        }
    }

    private void saveCopyToDocument() {
        if (handle == 0) return;
        if (pendingPicker != null) {
            fileMessage("Finish the file picker already open.");
            return;
        }
        launchSavePicker(true);
    }

    private void launchSavePicker(boolean copy) {
        Intent intent = new Intent(Intent.ACTION_CREATE_DOCUMENT);
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        intent.setType("application/octet-stream");
        String name = documentUri == null || documentUri.getLastPathSegment() == null
                ? "document.mathed" : documentUri.getLastPathSegment();
        if (!name.endsWith(".mathed")) name = name + ".mathed";
        if (copy) name = name.replaceFirst("\\.mathed$", " copy.mathed");
        intent.putExtra(Intent.EXTRA_TITLE, name);
        intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
        int request = copy ? SAVE_COPY_DOCUMENT : SAVE_DOCUMENT;
        launchPicker(intent, request,
                new PendingPicker(request, session.capture(PadSession.FILE, copy ? "copy-picker" : "save-picker"), copy));
    }

    private void launchPicker(Intent intent, int requestCode, PendingPicker picker) {
        pendingPicker = picker;
        try {
            startActivityForResult(intent, requestCode);
        } catch (RuntimeException e) {
            pendingPicker = null;
            fileMessage("Could not open a file picker: " + e.getMessage());
        }
    }

    @Override protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode == EXPORT_RECOVERY) {
            if (resultCode != RESULT_OK || data == null || data.getData() == null) return;
            exportRecoveryBytes(data.getData());
            return;
        }
        PendingPicker picker = pendingPicker;
        if (picker == null || picker.requestCode != requestCode) {
            if (requestCode == OPEN_DOCUMENT || requestCode == SAVE_DOCUMENT
                    || requestCode == SAVE_COPY_DOCUMENT) {
                fileMessage("File request expired; try again.");
            }
            return;
        }
        pendingPicker = null;
        if (resultCode != RESULT_OK || data == null || data.getData() == null) {
            fileMessage("Cancelled; the current document is unchanged.");
            return;
        }
        if (!session.sameDocument(picker.owner)) {
            fileMessage("The document changed; that file request was ignored.");
            return;
        }
        final Uri uri = data.getData();
        int flags = data.getFlags() & (Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
        try { getContentResolver().takePersistableUriPermission(uri, flags); }
        catch (SecurityException e) { Log.w("emthin-pad", "document access is temporary", e); }
        if (requestCode == OPEN_DOCUMENT) readDocument(uri, picker.owner);
        else beginSave(uri, picker.copy, false, false);
    }

    private void exportRecoveryBytes(final Uri uri) {
        final byte[] bytes = recoveryExport;
        if (bytes == null) return;
        fileWorker.execute(new Runnable() {
            @Override public void run() {
                try (java.io.OutputStream out = getContentResolver().openOutputStream(uri, "wt")) {
                    ProviderIo.writeFully(out, bytes);
                    main.post(new Runnable() { @Override public void run() {
                        fileMessage("Recovery data exported");
                    } });
                } catch (Exception e) {
                    main.post(new Runnable() { @Override public void run() {
                        fileMessage("Could not export recovery: " + e.getMessage());
                    } });
                }
            }
        });
    }

    private void rememberDocument(Uri uri, boolean snapshot, ProviderIo.Stamp stamp) {
        documentUri = uri;
        documentIsSnapshot = snapshot;
        fileStamp = stamp;
        SharedPreferences.Editor edit = prefs().edit()
                .putString(PREF_DOCUMENT_URI, uri.toString())
                .putBoolean(PREF_DOCUMENT_SNAPSHOT, snapshot);
        if (stamp != null && stamp.known()) {
            edit.putString(PREF_DOCUMENT_HASH, stamp.sha256)
                    .putLong(PREF_DOCUMENT_SIZE, stamp.size)
                    .putLong(PREF_DOCUMENT_MODIFIED, stamp.modified);
        } else {
            edit.remove(PREF_DOCUMENT_HASH).remove(PREF_DOCUMENT_SIZE).remove(PREF_DOCUMENT_MODIFIED);
        }
        edit.apply();
    }

    private void forgetDocument() {
        documentUri = null;
        documentIsSnapshot = true;
        fileStamp = null;
        prefs().edit()
                .remove(PREF_DOCUMENT_URI)
                .remove(PREF_DOCUMENT_SNAPSHOT)
                .remove(PREF_DOCUMENT_HASH)
                .remove(PREF_DOCUMENT_SIZE)
                .remove(PREF_DOCUMENT_MODIFIED)
                .apply();
    }

    private void readDocument(final Uri uri, PadSession.Owner pickerOwner) {
        if (handle == 0) return;
        FileOperation op = new FileOperation("open", pickerOwner, uri, null, false,
                false, false, false, null, DocBridge.snapshot(handle));
        enqueueFileOperation(op);
    }

    private void beginSave(final Uri uri, boolean copy, boolean replaceConfirmed,
                           boolean recheckBeforeReplace) {
        if (handle == 0) return;
        finishActiveComposition();
        final byte[] bytes;
        try {
            String encoded = DocBridge.snapshot(handle);
            if (encoded == null || encoded.isEmpty()) throw new java.io.IOException("No snapshot");
            bytes = Base64.decode(encoded, Base64.DEFAULT);
        } catch (RuntimeException | java.io.IOException e) {
            fileMessage("Could not capture the document: " + e.getMessage());
            return;
        }
        boolean activeDocument = !copy && documentUri != null && uri.equals(documentUri);
        FileOperation op = new FileOperation("save", session.capture(PadSession.FILE, uri.toString()),
                uri, bytes, copy, replaceConfirmed, recheckBeforeReplace,
                activeDocument, activeDocument ? fileStamp : null, null);
        enqueueFileOperation(op);
    }

    private void enqueueFileOperation(FileOperation op) {
        if (op.cancelled || handle == 0) return;
        if (activeFile != null) {
            if (!op.copy && !activeFile.copy && op.uri.equals(activeFile.uri)
                    && "save".equals(op.kind) && "save".equals(activeFile.kind)) {
                owedFileSave = op;
                status.setText("Save queued · the newest capture will be written");
            } else if (waitingFile == null) {
                waitingFile = op;
                status.setText("Waiting for the current file operation…");
            } else {
                fileMessage("Another file operation is already waiting; try again when it finishes.");
            }
            return;
        }
        startFileOperation(op);
    }

    private void startFileOperation(final FileOperation op) {
        activeFile = op;
        status.setText("save".equals(op.kind)
                ? (op.copy ? "Saving copy…" : "Saving document…")
                : "Opening document…");
        main.removeCallbacks(showFileCancel);
        main.postDelayed(showFileCancel, 1000);
        op.future = fileWorker.submit(new Runnable() {
            @Override public void run() {
                if ("open".equals(op.kind)) runOpen(op);
                else runSave(op);
            }
        });
    }

    private void runOpen(final FileOperation op) {
        byte[] bytes = null;
        String failure = null;
        try (java.io.InputStream input = getContentResolver().openInputStream(op.uri)) {
            op.stream = input;
            bytes = ProviderIo.readBounded(input, ProviderIo.MAX_DOCUMENT_BYTES);
            op.stream = null;
        } catch (Exception e) {
            failure = e.getMessage() == null ? e.toString() : e.getMessage();
        }
        final byte[] data = bytes;
        final String error = failure;
        final String fingerprint = data == null ? "" : DocBridge.logicalFingerprint(data);
        final String contentHash = data == null ? "" : ProviderIo.sha256(data);
        final long modified = data == null ? 0 : providerModified(op.uri);
        main.post(new Runnable() {
            @Override public void run() {
                finishFileOperation(op, new Runnable() {
                    @Override public void run() {
                        if (error != null) {
                            if (session.sameDocument(op.owner))
                                fileMessage("Could not open document: " + error);
                            return;
                        }
                        openDownloaded(op, data, fingerprint, contentHash, modified);
                    }
                });
            }
        });
    }

    private void openDownloaded(FileOperation op, byte[] bytes, String fingerprint,
                                String contentHash, long modified) {
        if (handle == 0 || !session.sameDocument(op.owner)) return;
        if (op.originalSnapshot == null || !op.originalSnapshot.equals(DocBridge.snapshot(handle))) {
            fileMessage("Document changed while opening; open it again.");
            return;
        }
        long next = fingerprint == null || fingerprint.isEmpty()
                ? 0 : DocBridge.openSnapshotBytes(bytes);
        final boolean snapshot = next != 0;
        String fileName = op.uri.getLastPathSegment();
        boolean snapshotRequired = ProviderIo.looksLikeSnapshot(bytes)
                || (fileName != null && fileName.toLowerCase(java.util.Locale.ROOT).endsWith(".mathed"));
        if (!snapshot) {
            if (snapshotRequired) {
                fileMessage("Could not open this document: malformed .mathed snapshot.");
                return;
            }
            try {
                String text = ProviderIo.decodeUtf8(bytes);
                next = DocBridge.create(text);
            } catch (java.nio.charset.CharacterCodingException e) {
                fileMessage("Could not open this document: invalid UTF-8.");
                return;
            }
        }
        if (next == 0) {
            fileMessage("Could not open this document.");
            return;
        }
        final long opened = next;
        final String openedFingerprint = snapshot ? fingerprint : "";
        final PadSession.Owner openOwner = session.capture(PadSession.OPEN, op.uri.toString());
        final ProviderIo.Stamp stamp = snapshot
                ? new ProviderIo.Stamp(contentHash, bytes.length, modified) : null;
        checkpointThen(new Runnable() {
            @Override public void run() {
                if (handle == 0 || !session.isCurrent(openOwner)
                        || op.originalSnapshot == null
                        || !op.originalSnapshot.equals(DocBridge.snapshot(handle))) {
                    DocBridge.free(opened);
                    fileMessage("Document changed while opening; open it again.");
                    return;
                }
                if (inspectorOpen) closeInspector();
                if (searchBar.getVisibility() == View.VISIBLE) closeSearch(false);
                long old = handle;
                handle = opened;
                DocBridge.free(old);
                session.beginDocument();
                renderWork.clearPending();
                expandedSegments.clear();
                clearSelectionAnchor();
                restoreScrollY = -1;
                firstRender = true;
                if (snapshot) {
                    rememberDocument(op.uri, true, stamp);
                    savedFingerprint = openedFingerprint;
                    unsaved = false;
                } else {
                    // A plain source file has no marks. Keep it unsaved and
                    // require Save to create a .mathed snapshot.
                    forgetDocument();
                    savedFingerprint = "";
                    unsaved = true;
                }
                durableFingerprint = "";
                restoringState = true;
                applyState(DocBridge.state(handle), true);
                restoringState = false;
                fileMessage(snapshot ? "Document opened"
                        : "Source imported; Save creates a .mathed document");
            }
        });
    }

    private void runSave(final FileOperation op) {
        ProviderIo.Stamp remembered = op.baseline;
        if (op.activeDocument && !op.replaceConfirmed
                && (remembered == null || !remembered.known())) {
            main.post(new Runnable() { @Override public void run() {
                finishFileOperation(op, new Runnable() { @Override public void run() {
                    if (session.sameDocument(op.owner)) showFileChanged(op.uri, true);
                } });
            } });
            return;
        }
        if (!op.copy && !op.replaceConfirmed && remembered != null && remembered.known()) {
            try {
                ProviderIo.Stamp current = readDestinationStamp(op);
                if (!remembered.sameContent(current)) {
                    main.post(new Runnable() { @Override public void run() {
                        finishFileOperation(op, new Runnable() { @Override public void run() {
                            if (session.sameDocument(op.owner)) showFileChanged(op.uri, false);
                        } });
                    } });
                    return;
                }
            } catch (Exception e) {
                main.post(new Runnable() { @Override public void run() {
                    finishFileOperation(op, new Runnable() { @Override public void run() {
                        if (session.sameDocument(op.owner)) showFileChanged(op.uri, true);
                    } });
                } });
                return;
            }
        } else if (!op.copy && op.recheckBeforeReplace) {
            try {
                ProviderIo.Stamp current = readDestinationStamp(op);
                if (remembered != null && remembered.known() && !remembered.sameContent(current)) {
                    main.post(new Runnable() { @Override public void run() {
                        finishFileOperation(op, new Runnable() { @Override public void run() {
                            if (session.sameDocument(op.owner)) showFileChanged(op.uri, false);
                        } });
                    } });
                    return;
                }
            } catch (Exception e) {
                main.post(new Runnable() { @Override public void run() {
                    finishFileOperation(op, new Runnable() { @Override public void run() {
                        if (session.sameDocument(op.owner)) showFileChanged(op.uri, true);
                    } });
                } });
                return;
            }
        }
        if (op.cancelled) return;
        try (java.io.OutputStream out = getContentResolver().openOutputStream(op.uri, "wt")) {
            op.stream = out;
            ProviderIo.writeFully(out, op.bytes);
            op.stream = null;
        } catch (Exception e) {
            final String error = e.getMessage() == null ? e.toString() : e.getMessage();
            main.post(new Runnable() { @Override public void run() {
                finishFileOperation(op, new Runnable() { @Override public void run() {
                    saveFailed(op, error);
                } });
            } });
            return;
        }
        final ProviderIo.Stamp written = ProviderIo.Stamp.of(op.bytes, providerModified(op.uri));
        final String fingerprint = DocBridge.logicalFingerprint(op.bytes);
        main.post(new Runnable() { @Override public void run() {
            finishFileOperation(op, new Runnable() { @Override public void run() {
                saveSucceeded(op, written, fingerprint);
            } });
        } });
    }

    private ProviderIo.Stamp readDestinationStamp(FileOperation op) throws java.io.IOException {
        try (java.io.InputStream input = getContentResolver().openInputStream(op.uri)) {
            op.stream = input;
            byte[] current = ProviderIo.readBounded(input, ProviderIo.MAX_DOCUMENT_BYTES);
            op.stream = null;
            return ProviderIo.Stamp.of(current, providerModified(op.uri));
        }
    }

    private long providerModified(Uri uri) {
        try (android.database.Cursor cursor = getContentResolver().query(uri,
                new String[] {android.provider.DocumentsContract.Document.COLUMN_LAST_MODIFIED},
                null, null, null)) {
            if (cursor != null && cursor.moveToFirst() && !cursor.isNull(0)) return cursor.getLong(0);
        } catch (RuntimeException ignored) { }
        return 0;
    }

    private void finishFileOperation(FileOperation op, Runnable then) {
        if (activeFile == op) {
            activeFile = null;
            main.removeCallbacks(showFileCancel);
            dismissFileCancel();
        }
        if (then != null && !op.cancelled) then.run();
        startWaitingFileOperation();
    }

    private void startWaitingFileOperation() {
        if (activeFile != null) return;
        FileOperation next = owedFileSave;
        owedFileSave = null;
        if (next == null) {
            next = waitingFile;
            waitingFile = null;
        }
        if (next == null) return;
        if (!session.sameDocument(next.owner)) {
            startWaitingFileOperation();
            return;
        }
        status.setText("Starting queued file operation…");
        startFileOperation(next);
    }

    private void saveSucceeded(FileOperation op, ProviderIo.Stamp stamp, String fingerprint) {
        if (!session.sameDocument(op.owner)) return;
        if (op.copy) {
            fileMessage("Copy saved · active document unchanged");
            return;
        }
        boolean clean = session.isCurrent(op.owner)
                && fingerprint != null && !fingerprint.isEmpty();
        if (owedFileSave != null && owedFileSave.uri.equals(op.uri))
            owedFileSave.baseline = stamp;
        rememberDocument(op.uri, true, stamp);
        savedFingerprint = fingerprint == null ? "" : fingerprint;
        unsaved = !clean;
        captureRecoveryNow();
        if (clean) {
            status.setText("Document saved · durable");
            fileMessage("Document saved");
        } else {
            fileMessage("An earlier revision was saved; newer edits remain unsaved.");
        }
    }

    private void saveFailed(FileOperation op, String error) {
        if (!session.sameDocument(op.owner)) return;
        if (op.copy) {
            fileMessage("Could not save copy: " + error + ". The active file and recovery are unchanged.");
            return;
        }
        unsaved = true;
        fileMessage("Could not save document: " + error + ". Recovery is retained.");
        new AlertDialog.Builder(this, android.R.style.Theme_Material_Dialog_Alert)
                .setTitle("Save failed")
                .setMessage("The file may be unchanged. The current document and its recovery copy are still here.")
                .setPositiveButton("Save copy", new DialogInterface.OnClickListener() {
                    @Override public void onClick(DialogInterface dialog, int which) {
                        saveCopyToDocument();
                    }
                })
                .setNegativeButton("Cancel", null).show();
    }

    private void showFileChanged(final Uri uri, final boolean unreadable) {
        if (handle == 0 || isDestroyed()) return;
        AlertDialog.Builder builder = new AlertDialog.Builder(this, android.R.style.Theme_Material_Dialog_Alert)
                .setTitle("File changed elsewhere")
                .setMessage(unreadable
                        ? "The destination cannot be checked. Save a copy, or replace it without outside-change protection."
                        : "The file differs from the version this editor opened. Save a copy, or replace it intentionally. The provider cannot make a replacement atomic; a concurrent writer may still win.")
                .setPositiveButton("Save copy", new DialogInterface.OnClickListener() {
                    @Override public void onClick(DialogInterface dialog, int which) {
                        saveCopyToDocument();
                    }
                })
                .setNegativeButton("Cancel", null);
        if (unreadable) {
            builder.setNeutralButton("Replace anyway", new DialogInterface.OnClickListener() {
                @Override public void onClick(DialogInterface dialog, int which) {
                    beginSave(uri, false, true, false);
                }
            });
        } else {
            builder.setNeutralButton("Replace file", new DialogInterface.OnClickListener() {
                @Override public void onClick(DialogInterface dialog, int which) {
                    beginSave(uri, false, true, true);
                }
            });
        }
        builder.show();
    }

    private void showFileCancelForActiveOperation() {
        final FileOperation op = activeFile;
        if (op == null || isDestroyed()) return;
        fileCancelDialog = new AlertDialog.Builder(this, android.R.style.Theme_Material_Dialog_Alert)
                .setTitle("File operation still running")
                .setMessage("Cancelling stops this result from being used. Some providers may have already changed the destination.")
                .setNegativeButton("Keep waiting", null)
                .setPositiveButton("Cancel", new DialogInterface.OnClickListener() {
                    @Override public void onClick(DialogInterface dialog, int which) {
                        cancelFileOperation(op);
                    }
                }).show();
    }

    private void cancelFileOperation(FileOperation op) {
        op.cancelled = true;
        if (op.future != null) op.future.cancel(true);
        try { if (op.stream != null) op.stream.close(); }
        catch (java.io.IOException ignored) { }
        if (activeFile == op) {
            activeFile = null;
            main.removeCallbacks(showFileCancel);
            dismissFileCancel();
            status.setText("save".equals(op.kind)
                    ? "Write cancelled · the destination may have changed · recovery retained"
                    : "Open cancelled · recovery retained");
        }
        startWaitingFileOperation();
    }

    private void dismissFileCancel() {
        if (fileCancelDialog != null) {
            fileCancelDialog.dismiss();
            fileCancelDialog = null;
        }
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
        if (handle == 0) return;
        finishActiveComposition();
        checkpointThen(new Runnable() {
            @Override public void run() { newDocumentNow(); }
        });
    }

    private void newDocumentNow() {
        if (handle == 0) {
            return;
        }
        finishActiveComposition();
        forgetDocument();
        session.beginDocument();
        renderWork.clearPending();
        expandedSegments.clear();
        clearSelectionAnchor();
        readingAnchorStable = null;
        readingAnchorFallback = -1;
        scrollToReadingAnchorAfterChunk = false;
        // A New document starts over instead of inheriting the old reading
        // position if the process dies before onPause saves the new one.
        prefs().edit()
                .remove(PREF_CARET)
                .remove(PREF_SCROLL)
                .remove(PREF_SCROLL_WIDTH)
                .apply();
        firstRender = true;
        durableFingerprint = "";
        savedFingerprint = "";
        unsaved = true;
        restoringState = true;
        applyState(DocBridge.sync(handle, STARTER, 0), true);
        restoringState = false;
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
    private final class CaretEditText extends CompositionEditText {
        CaretEditText(android.content.Context context) {
            super(context);
        }

        @Override protected void compositionCommitted() {
            if (this == editor) commitEditorText(getText());
        }

        @Override
        protected void onSelectionChanged(int selStart, int selEnd) {
            super.onSelectionChanged(selStart, selEnd);
            // `applying` guards the programmatic setText in applyState
            // (which schedules its own render); handle == 0 is the
            // pre-create window before the model exists.
            if (!applying && !syncingCaret && handle != 0) {
                // Caret movement is overlay-only in the rendered editor: the
                // layout key excludes it, so repaint rather than recompile.
                updateModifierPanel();
            }
            invalidateRenderedCaret();
        }
    }

    private final class SourceEditText extends CompositionEditText {
        SourceEditText(android.content.Context context) { super(context); }
        @Override protected void compositionCommitted() {
            if (this == sourceField) commitSourceEdit(this, getText());
        }
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

    private static boolean isComposing(EditText field) {
        return field instanceof CompositionEditText && ((CompositionEditText) field).isComposing();
    }

    private void finishActiveComposition() {
        if (editor instanceof CompositionEditText) {
            ((CompositionEditText) editor).finishComposition();
        }
        if (sourceField instanceof CompositionEditText) {
            ((CompositionEditText) sourceField).finishComposition();
        }
    }

    private void setDocumentCaret(int offset) {
        if (editor == null) return;
        int caret = Math.max(0, Math.min(editor.length(), offset));
        syncingCaret = true;
        try { editor.setSelection(caret); } finally { syncingCaret = false; }
        trace("caretMove", "offset", caret);
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

    private static final class AccessibleBlock {
        final int start, end, citation;
        final String text;
        final android.graphics.Rect bounds;
        AccessibleBlock(int start, int end, int citation, String text,
                android.graphics.Rect bounds) {
            this.start = start;
            this.end = end;
            this.citation = citation;
            this.text = text;
            this.bounds = bounds;
        }
    }

    /**
     * Group page hits into paragraph-sized source ranges. Bounds stay in page
     * pixels and come from the same PageTransform as drawing and taps.
     */
    private List<AccessibleBlock> accessibleBlocks(RenderedPage page) {
        List<AccessibleBlock> blocks = new ArrayList<>();
        if (!page.loaded || editor == null || page.hits.length() == 0) return blocks;
        String text = editor.getText().toString();
        float scale = page.pointScale();
        int blockStart = -1, previousOffset = -1, citation = 0;
        float lineHeight = 0f, lastBottom = -1f;
        android.graphics.Rect bounds = null;
        for (int i = 0; i < page.hits.length(); i++) {
            JSONArray hit = page.hits.optJSONArray(i);
            if (hit == null) continue;
            int offset = hit.optInt(4, -1);
            if (offset < 0 || offset > text.length()) continue;
            float x1 = (float) hit.optDouble(0) * scale;
            float y1 = (float) hit.optDouble(1) * scale;
            float x2 = (float) hit.optDouble(2) * scale;
            float y2 = (float) hit.optDouble(3) * scale;
            float height = y2 - y1;
            boolean split = blockStart < 0;
            if (!split && lineHeight > 0 && y1 - lastBottom > lineHeight * 1.8f) split = true;
            if (!split && previousOffset >= 0 && offset > previousOffset) {
                int gapEnd = Math.min(text.length(), offset);
                if (gapEnd > previousOffset
                        && text.substring(previousOffset, gapEnd).contains("\n\n")) split = true;
            }
            if (split) {
                if (blockStart >= 0 && bounds != null) {
                    int end = snapGrapheme(Math.min(text.length(), previousOffset + 1), 1);
                    blocks.add(new AccessibleBlock(blockStart, end, citation,
                            visibleSource(blockStart, end), bounds));
                }
                blockStart = offset;
                citation = 0;
                bounds = new android.graphics.Rect(Math.round(x1), Math.round(y1),
                        Math.round(x2), Math.round(y2));
                lineHeight = height;
                lastBottom = y2;
            } else {
                bounds.union(Math.round(x1), Math.round(y1),
                        Math.round(x2), Math.round(y2));
                lastBottom = Math.max(lastBottom, y2);
                if (lineHeight <= 0) lineHeight = height;
            }
            if (citation <= 0) citation = citationAt(offset);
            previousOffset = offset;
        }
        if (blockStart >= 0 && bounds != null) {
            int end = snapGrapheme(Math.min(text.length(), previousOffset + 1), 1);
            blocks.add(new AccessibleBlock(blockStart, end, citation,
                    visibleSource(blockStart, end), bounds));
        }
        return blocks;
    }

    private void activateAccessibleBlock(AccessibleBlock block) {
        if (block == null) return;
        if (block.citation > 0) { showCitations(block.citation); return; }
        if (toggleFoldAt(block.start, true)) return;
        clearSelectionAnchor();
        if (targetsField != null) commitTargetPairs(targetsField, false);
        inspectorOpen = true;
        wholeDocumentEditing = false;
        inspectorSignature = "";
        editor.setSelection(Math.max(0, Math.min(editor.length(), block.start)));
        updateModifierPanel();
        focusSourceCaret();
    }

    private final class RenderedPage extends ImageView {
        final int index;
        JSONArray hits = new JSONArray();
        TextView caption;
        double textTop = -1;
        double widthPt = PAGE_WIDTH_PT;
        double heightPt = 841.89;
        boolean hasFigures;
        boolean loaded;
        long bitmapBytes;
        long lastVisible;
        final Paint caretPaint = new Paint(Paint.ANTI_ALIAS_FLAG);
        RenderedPage(android.content.Context context, int index) {
            super(context);
            this.index = index;
            caretPaint.setColor(Color.parseColor("#B3CFE4"));
            caretPaint.setStyle(Paint.Style.STROKE);
            caretPaint.setStrokeWidth(Math.max(2f, getResources().getDisplayMetrics().density));
            setFocusable(true);
            setFocusableInTouchMode(true);
        }
        float pointScale() {
            return (float) (getWidth() / Math.max(1.0, widthPt));
        }

        private final android.view.accessibility.AccessibilityNodeProvider provider =
                new android.view.accessibility.AccessibilityNodeProvider() {
            @Override public android.view.accessibility.AccessibilityNodeInfo
                    createAccessibilityNodeInfo(int virtualViewId) {
                if (virtualViewId == HOST_VIEW_ID) return hostNode();
                List<AccessibleBlock> blocks = accessibleBlocks(RenderedPage.this);
                if (virtualViewId < 0 || virtualViewId >= blocks.size()) return null;
                return blockNode(virtualViewId, blocks.get(virtualViewId));
            }
            @Override public boolean performAction(int virtualViewId, int action,
                    android.os.Bundle arguments) {
                if (!loaded) return false;
                if (virtualViewId == HOST_VIEW_ID) return false;
                List<AccessibleBlock> blocks = accessibleBlocks(RenderedPage.this);
                if (virtualViewId < 0 || virtualViewId >= blocks.size()) return false;
                AccessibleBlock block = blocks.get(virtualViewId);
                if (action == android.view.accessibility.AccessibilityNodeInfo.ACTION_CLICK) {
                    activateAccessibleBlock(block);
                    sendAccessibilityNodeEvent(virtualViewId,
                            android.view.accessibility.AccessibilityEvent.TYPE_VIEW_CLICKED, block);
                    return true;
                }
                if (action == android.view.accessibility.AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS) {
                    accessibilityFocusOffset = block.start;
                    invalidate();
                    sendAccessibilityNodeEvent(virtualViewId,
                            android.view.accessibility.AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUSED, block);
                    return true;
                }
                if (action == android.view.accessibility.AccessibilityNodeInfo.ACTION_CLEAR_ACCESSIBILITY_FOCUS) {
                    if (accessibilityFocusOffset >= block.start
                            && accessibilityFocusOffset < Math.max(block.start + 1, block.end)) {
                        accessibilityFocusOffset = -1;
                    }
                    invalidate();
                    sendAccessibilityNodeEvent(virtualViewId,
                            android.view.accessibility.AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED, block);
                    return true;
                }
                return false;
            }
            @Override public android.view.accessibility.AccessibilityNodeInfo findFocus(int focus) {
                if (focus != android.view.accessibility.AccessibilityNodeInfo.FOCUS_ACCESSIBILITY
                        || accessibilityFocusOffset < 0) return null;
                List<AccessibleBlock> blocks = accessibleBlocks(RenderedPage.this);
                for (int i = 0; i < blocks.size(); i++) {
                    AccessibleBlock block = blocks.get(i);
                    if (accessibilityFocusOffset >= block.start
                            && accessibilityFocusOffset <= block.end) {
                        return blockNode(i, block);
                    }
                }
                return null;
            }
        };

        @Override public android.view.accessibility.AccessibilityNodeProvider
                getAccessibilityNodeProvider() {
            return provider;
        }

        private android.graphics.Rect screenBounds() {
            int[] location = new int[2];
            getLocationOnScreen(location);
            return new android.graphics.Rect(location[0], location[1],
                    location[0] + getWidth(), location[1] + getHeight());
        }

        private boolean pageVisibleToUser() {
            if (!isShown() || getWindowVisibility() != View.VISIBLE) return false;
            android.graphics.Rect visible = new android.graphics.Rect();
            return getLocalVisibleRect(visible) && !visible.isEmpty();
        }

        private android.view.accessibility.AccessibilityNodeInfo hostNode() {
            android.view.accessibility.AccessibilityNodeInfo info =
                    new android.view.accessibility.AccessibilityNodeInfo();
            info.setSource(RenderedPage.this);
            info.setPackageName(getPackageName());
            info.setClassName("android.widget.ImageView");
            info.setEnabled(true);
            info.setFocusable(true);
            info.setClickable(true);
            info.setBoundsInScreen(screenBounds());
            info.setVisibleToUser(pageVisibleToUser());
            info.setContentDescription("Page " + (index + 1) + " of " + Math.max(totalPages, index + 1)
                    + (loaded ? "" : ", loading"));
            if (loaded) {
                List<AccessibleBlock> blocks = accessibleBlocks(RenderedPage.this);
                for (int i = 0; i < blocks.size(); i++) info.addChild(RenderedPage.this, i);
            } else if (!pagesStale && renderOwner != null) {
                // Accessibility focus can reach a placeholder before the
                // visual viewport does. Load just that page, never the
                // intervening bitmaps.
                main.post(new Runnable() {
                    @Override public void run() {
                        if (!loaded && !pagesStale) requestPage(index, "accessible");
                    }
                });
            }
            return info;
        }

        private android.view.accessibility.AccessibilityNodeInfo blockNode(
                int id, AccessibleBlock block) {
            android.view.accessibility.AccessibilityNodeInfo info =
                    new android.view.accessibility.AccessibilityNodeInfo();
            info.setSource(RenderedPage.this, id);
            info.setParent(RenderedPage.this);
            info.setPackageName(getPackageName());
            info.setClassName("android.widget.TextView");
            info.setText(block.text);
            info.setContentDescription(block.text);
            info.setEnabled(true);
            info.setFocusable(true);
            info.setClickable(true);
            int[] location = new int[2];
            getLocationOnScreen(location);
            android.graphics.Rect local = new android.graphics.Rect(block.bounds);
            info.setBoundsInScreen(new android.graphics.Rect(local.left + location[0],
                    local.top + location[1], local.right + location[0],
                    local.bottom + location[1]));
            android.graphics.Rect visible = new android.graphics.Rect();
            info.setVisibleToUser(pageVisibleToUser()
                    && getLocalVisibleRect(visible) && android.graphics.Rect.intersects(visible, local));
            boolean focused = accessibilityFocusOffset >= block.start
                    && accessibilityFocusOffset <= block.end;
            info.setAccessibilityFocused(focused);
            info.setSelected(false);
            info.addAction(android.view.accessibility.AccessibilityNodeInfo.AccessibilityAction.ACTION_CLICK);
            info.addAction(focused
                    ? android.view.accessibility.AccessibilityNodeInfo.AccessibilityAction.ACTION_CLEAR_ACCESSIBILITY_FOCUS
                    : android.view.accessibility.AccessibilityNodeInfo.AccessibilityAction.ACTION_ACCESSIBILITY_FOCUS);
            if (block.citation > 0) {
                info.setContentDescription(block.text + ". Citation. Activate to open.");
            }
            return info;
        }

        private void sendAccessibilityNodeEvent(int id, int type, AccessibleBlock block) {
            android.view.accessibility.AccessibilityEvent event =
                    new android.view.accessibility.AccessibilityEvent();
            event.setEventType(type);
            event.setSource(RenderedPage.this, id);
            event.setPackageName(getPackageName());
            event.setClassName("android.widget.TextView");
            event.getText().add(block.text);
            if (getParent() != null) getParent().requestSendAccessibilityEvent(RenderedPage.this, event);
        }

        @Override protected void onDraw(Canvas canvas) {
            super.onDraw(canvas);
            if (editor == null) return;
            if (selectionAnchor >= 0 && selectionAnchor != editor.getSelectionStart()) {
                int begin = Math.min(selectionAnchor, editor.getSelectionStart());
                int end = Math.max(selectionAnchor, editor.getSelectionStart());
                Paint selected = new Paint();
                selected.setColor(0x88628AB3);
                float factor = pointScale();
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
                float factor = pointScale();
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
            float scale = pointScale();
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
                    float scale = current.page.pointScale();
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
                    int end = DocBridge.graphemeBoundary(editor.getText().toString(), offset,
                            offset == current ? 2 : 1);
                    if (direction > 0 && end > current) candidate = Math.min(candidate, end);
                    if (direction < 0 && end < current) candidate = Math.max(candidate, end);
                }
            }
        }
        if (candidate != Integer.MAX_VALUE && candidate >= 0) {
            candidate = snapGrapheme(candidate, direction > 0 ? 1 : 0);
            setDocumentCaret(candidate);
        }
        focusRenderedCaret();
    }

    private int snapGrapheme(int offset, int direction) {
        if (editor == null) return Math.max(0, offset);
        return DocBridge.graphemeBoundary(editor.getText().toString(),
                Math.max(0, Math.min(editor.length(), offset)), direction);
    }

    private void moveRenderedCaretVertical(int direction) {
        CaretSpot current = caretSpot(editor.getSelectionStart());
        if (current.page == null) return;
        float scale = current.page.pointScale();
        float x = (float) current.hit.optDouble(0) * scale;
        float y = current.page.getTop() + (float) current.hit.optDouble(1) * scale;
        double best = Double.MAX_VALUE;
        int candidate = -1;
        for (RenderedPage page : renderedPages) {
            float pageScale = page.pointScale();
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
        if (pagesStale && getCurrentFocus() instanceof RenderedPage) {
            // The old frame remains visible, but its hit/source mapping is for
            // a superseded capture. Reject edits until the replacement chunk
            // is accepted instead of mutating through stale positions.
            return true;
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
                    if (event.isShiftPressed() && selectionAnchor < 0) setSelectionAnchor(editor.getSelectionStart());
                    if (!event.isShiftPressed()) clearSelectionAnchor();
                    moveRenderedCaretVertical(direction);
                    return true;
                }
            }
            if ((code == KeyEvent.KEYCODE_MOVE_HOME || code == KeyEvent.KEYCODE_MOVE_END)
                    && getCurrentFocus() instanceof RenderedPage) {
                clearSelectionAnchor();
                setDocumentCaret(code == KeyEvent.KEYCODE_MOVE_HOME ? 0 : editor.length());
                focusRenderedCaret();
                return true;
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
                    if (event.isShiftPressed() && selectionAnchor < 0) setSelectionAnchor(editor.getSelectionStart());
                    if (!event.isShiftPressed() && selectionAnchor >= 0) {
                        int edge = direction < 0 ? Math.min(selectionAnchor, editor.getSelectionStart())
                                : Math.max(selectionAnchor, editor.getSelectionStart());
                        clearSelectionAnchor();
                        setDocumentCaret(edge);
                        focusRenderedCaret();
                        return true;
                    }
                    moveRenderedCaret(direction);
                    return true;
                }
            }
            if ((code == KeyEvent.KEYCODE_DEL || code == KeyEvent.KEYCODE_FORWARD_DEL)
                    && getCurrentFocus() instanceof RenderedPage) {
                if (hasRenderedSelection()) deleteRenderedSelection();
                else deleteRenderedGrapheme(code == KeyEvent.KEYCODE_DEL ? -1 : 1);
                return true;
            }
            if (getCurrentFocus() instanceof RenderedPage && event.getUnicodeChar() >= 32) {
                String typed = new String(Character.toChars(event.getUnicodeChar()));
                int[] range = selectionRange();
                if (range != null) replaceRenderedRange(range[0], range[1], typed);
                inspectorOpen = true;
                wholeDocumentEditing = false;
                inspectorSignature = "";
                updateModifierPanel();
                if (sourceField != null) {
                    focusSourceCaret();
                    if (range == null) {
                        sourceField.getText().insert(sourceField.getSelectionStart(), typed);
                    }
                    return true;
                }
                if (range != null) return true;
            }
        }
        if (event.getAction() == KeyEvent.ACTION_DOWN && event.isCtrlPressed()
                && !event.isAltPressed() && getCurrentFocus() instanceof RenderedPage) {
            switch (event.getKeyCode()) {
                case KeyEvent.KEYCODE_A:
                    setSelectionAnchor(0);
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
            if (event.isShiftPressed() && selectionAnchor < 0) setSelectionAnchor(editor.getSelectionStart());
            if (!event.isShiftPressed()) clearSelectionAnchor();
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
                    case KeyEvent.KEYCODE_S:
                        if (event.isShiftPressed()) saveCopyToDocument();
                        else saveToDocument();
                        return true;
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
                    float scale = page.pointScale();
                    int x = Math.round((float) hit.optDouble(0) * scale);
                    page.requestRectangleOnScreen(new android.graphics.Rect(x,
                            Math.round((float) hit.optDouble(1) * scale), x + dp(12),
                            Math.round((float) hit.optDouble(3) * scale)), false);
                }
            });
        } else {
            locatePage(target, "search");
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
        final PadSession.Owner dialogOwner =
                session.capture(PadSession.CITATION, "citations");
        final String text = editor.getText().toString();
        ScrollView scroll = new ScrollView(this);
        LinearLayout rows = new LinearLayout(this);
        rows.setOrientation(LinearLayout.VERTICAL);
        rows.setPadding(dp(12), dp(8), dp(12), dp(12));
        scroll.addView(rows);
        final AlertDialog dialog = new AlertDialog.Builder(this, android.R.style.Theme_Material_Dialog_Alert)
                .setTitle("Citations").setView(scroll).create();
        appendCitationRows(rows, text, dialog, dialogOwner, 0, openNumber);
        dialog.show();
    }

    private void appendCitationRows(LinearLayout rows, final String text, final AlertDialog dialog,
                                    final PadSession.Owner dialogOwner,
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
                                renderWork.submit("citation", RenderController.EXPLICIT, new RenderController.Job() {
                                    @Override public void run() {
                                        final String result = DocBridge.citationDetail(text, number, width, scale);
                                        main.post(new Runnable() {
                                            @Override public void run() {
                                                if (!dialog.isShowing()
                                                        || !session.sameDocument(dialogOwner)) return;
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
                                                        if (depth < 7) appendCitationRows(details, detail.optString("text"), dialog, dialogOwner, depth + 1, 0);
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
        return selectionRange() != null;
    }

    /** One grapheme-safe, marker-safe range for copy, cut and replacement. */
    private StableAnchor captureStableAnchor(int offset, int side) {
        if (handle == 0) return null;
        try {
            JSONObject anchor = new JSONObject(DocBridge.createAnchor(handle, offset, side));
            String encoded = anchor.optString("encoded", "");
            if (encoded.isEmpty()) return null;
            return new StableAnchor(session.documentId(), encoded,
                    anchor.optInt("side", side), anchor.optInt("fallback", offset));
        } catch (Exception e) {
            return null;
        }
    }

    private void setSelectionAnchor(int offset) {
        selectionAnchor = Math.max(0, Math.min(editor.length(), offset));
        selectionAnchorStable = captureStableAnchor(selectionAnchor, -1);
    }

    private void clearSelectionAnchor() {
        selectionAnchor = -1;
        selectionAnchorStable = null;
        hideSelectionBar();
    }

    /** Keep the selection's fixed end attached to its original character. */
    private void refreshSelectionAnchor() {
        if (selectionAnchorStable == null) return;
        if (!selectionAnchorStable.belongsTo(session.documentId())) {
            clearSelectionAnchor();
            return;
        }
        int resolved = selectionAnchorStable.resolve(handle, session.documentId());
        if (resolved < 0) {
            clearSelectionAnchor();
        } else {
            selectionAnchor = Math.max(0, Math.min(editor.length(), resolved));
        }
    }

    /** The source offset of the first visible rendered text band. */
    private int readingTextOffset() {
        if (renderedPages.isEmpty()) return -1;
        int y = previewScroll.getScrollY() + dp(4);
        int bestOffset = -1;
        double bestDistance = Double.MAX_VALUE;
        for (RenderedPage page : renderedPages) {
            int pageTop = page.getTop();
            float factor = page.pointScale();
            if (pageTop + page.getHeight() < y) continue;
            for (int i = 0; i < page.hits.length(); i++) {
                JSONArray hit = page.hits.optJSONArray(i);
                if (hit == null) continue;
                double top = pageTop + hit.optDouble(1) * factor;
                double bottom = pageTop + hit.optDouble(3) * factor;
                if (bottom < y) continue;
                double distance = Math.min(Math.abs(top - y), Math.abs(bottom - y));
                if (distance < bestDistance) {
                    bestDistance = distance;
                    bestOffset = hit.optInt(4, -1);
                }
            }
            if (bestOffset >= 0 && pageTop > y + dp(24)) break;
        }
        return bestOffset;
    }

    private void rememberReadingAnchor() {
        if (handle == 0 || pagesStale || renderedPages.isEmpty()) return;
        int offset = readingTextOffset();
        if (offset < 0) return;
        readingAnchorStable = captureStableAnchor(offset, -1);
        readingAnchorFallback = offset;
    }

    private void refreshReadingAnchor() {
        if (readingAnchorStable == null) return;
        if (!readingAnchorStable.belongsTo(session.documentId())) {
            readingAnchorStable = null;
            readingAnchorFallback = -1;
            return;
        }
        int resolved = readingAnchorStable.resolve(handle, session.documentId());
        if (resolved >= 0) readingAnchorFallback = resolved;
    }

    /** Scrolls to the stable reading anchor once its page is materialized. */
    private boolean scrollToReadingAnchor() {
        if (readingAnchorStable == null) return false;
        int offset = readingAnchorStable.resolve(handle, session.documentId());
        if (offset < 0) offset = readingAnchorFallback;
        if (offset < 0) return false;
        CaretSpot spot = caretSpot(offset);
        if (spot.page == null || spot.distance > 1) return false;
        float scale = spot.page.pointScale();
        int y = Math.max(0, spot.page.getTop()
                + Math.round((float) spot.hit.optDouble(1) * scale) - dp(4));
        previewScroll.scrollTo(0, y);
        restoreScrollY = -1;
        visibleScrollY = -1;
        restoreScrollWidth = 0;
        trace("readingAnchor", "offset", offset, "scrollY", y);
        return true;
    }

    private int[] selectionRange() {
        if (selectionAnchor < 0 || selectionAnchor == editor.getSelectionStart()) return null;
        int begin = snapGrapheme(Math.min(selectionAnchor, editor.getSelectionStart()), 0);
        int end = snapGrapheme(Math.max(selectionAnchor, editor.getSelectionStart()), 1);
        for (int i = 0; i < documentMarkers.length(); i++) {
            JSONObject marker = documentMarkers.optJSONObject(i);
            if (marker == null) continue;
            int left = marker.optInt("start", -1), right = marker.optInt("end", -1);
            right = Math.min(right, editor.length());
            if (left < 0 || right <= left) continue;
            if (begin > left && begin < right) begin = left;
            if (end > left && end < right) end = right;
        }
        if (begin >= end) return null;
        return new int[] {begin, end};
    }

    private String hiddenMarkersWithin(int begin, int end) {
        if (showMarkers) return "";
        StringBuilder preserved = new StringBuilder();
        String text = editor.getText().toString();
        for (int i = 0; i < documentMarkers.length(); i++) {
            JSONObject marker = documentMarkers.optJSONObject(i);
            if (marker == null) continue;
            int left = Math.max(begin, marker.optInt("start", -1));
            int right = Math.min(end, marker.optInt("end", -1));
            if (left >= begin && right > left && right <= text.length()) {
                preserved.append(text, left, right);
            }
        }
        return preserved.toString();
    }

    /** Replace a rendered range once, preserving hidden marker tokens. */
    private void replaceRenderedRange(int begin, int end, String inserted) {
        clearSelectionAnchor();
        int safeBegin = snapGrapheme(Math.max(0, Math.min(begin, end)), 0);
        int safeEnd = snapGrapheme(Math.max(begin, end), 1);
        String markers = hiddenMarkersWithin(safeBegin, safeEnd);
        editor.getText().replace(safeBegin, safeEnd, inserted + markers);
        setDocumentCaret(safeBegin + inserted.length());
    }

    /** Source text in [begin,end) with hidden marker tokens omitted. */
    private String visibleSource(int begin, int end) {
        String text = editor.getText().toString();
        begin = Math.max(0, Math.min(text.length(), begin));
        end = Math.max(begin, Math.min(text.length(), end));
        if (showMarkers) return text.substring(begin, end);
        StringBuilder visible = new StringBuilder();
        int at = begin;
        for (int i = 0; i < documentMarkers.length(); i++) {
            JSONObject marker = documentMarkers.optJSONObject(i);
            if (marker == null) continue;
            int left = marker.optInt("start", -1), right = marker.optInt("end", -1);
            if (right <= at || left >= end || left < 0) continue;
            if (left > at) visible.append(text, at, Math.min(left, end));
            at = Math.min(end, Math.max(at, right));
        }
        if (at < end) visible.append(text, at, end);
        return visible.toString();
    }

    private void copyRenderedSelection() {
        int[] range = selectionRange();
        if (range == null) return;
        android.content.ClipboardManager clipboard =
                (android.content.ClipboardManager) getSystemService(CLIPBOARD_SERVICE);
        if (clipboard != null) clipboard.setPrimaryClip(
                android.content.ClipData.newPlainText("", visibleSource(range[0], range[1])));
    }

    private void deleteRenderedSelection() {
        int[] range = selectionRange();
        clearSelectionAnchor();
        if (range == null) return;
        replaceRenderedRange(range[0], range[1], "");
    }

    private void deleteRenderedGrapheme(int direction) {
        clearSelectionAnchor();
        int caret = editor.getSelectionStart();
        int target = snapGrapheme(caret, direction < 0 ? -1 : 2);
        if (target == caret) return;
        editor.getText().replace(Math.min(caret, target), Math.max(caret, target), "");
        setDocumentCaret(Math.min(caret, target));
    }

    private void pasteRenderedClipboard() {
        android.content.ClipboardManager clipboard =
                (android.content.ClipboardManager) getSystemService(CLIPBOARD_SERVICE);
        if (clipboard == null || !clipboard.hasPrimaryClip() || clipboard.getPrimaryClip() == null
                || clipboard.getPrimaryClip().getItemCount() == 0) return;
        CharSequence value = clipboard.getPrimaryClip().getItemAt(0).coerceToText(this);
        if (value == null) return;
        int[] range = selectionRange();
        clearSelectionAnchor();
        pasting = true;
        try {
            if (range != null) {
                replaceRenderedRange(range[0], range[1], value.toString());
            } else {
                int caret = snapGrapheme(editor.getSelectionStart(), 0);
                editor.getText().insert(caret, value);
                setDocumentCaret(caret + value.length());
            }
        } finally { pasting = false; }
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
        if (sourceField instanceof CompositionEditText
                && ((CompositionEditText) sourceField).isComposing()) {
            // Rebuilding the field now would replace the buffer the IME is
            // composing into. Keep the candidate and refresh after commit.
            modifierRefreshDeferred = true;
            return;
        }
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
            @Override public void beforeTextChanged(CharSequence s, int st, int c, int a) {
                sourceCompletionAt = -1;
            }
            @Override public void onTextChanged(CharSequence s, int st, int b, int c) {
                if (!editingModifier && !isComposing(source) && b == 0 && c == 1) {
                    char ch = s.charAt(st);
                    boolean run = Character.isLetterOrDigit(ch) && ch < 128
                            || "\\-><=:|!_~".indexOf(ch) >= 0;
                    if (!run) sourceCompletionAt = st;
                }
            }
            @Override public void afterTextChanged(Editable s) {
                if (isComposing(source)) return;
                commitSourceEdit(source, s);
            }
        });
        controls.addView(source, new LinearLayout.LayoutParams(
                0, LinearLayout.LayoutParams.WRAP_CONTENT, 1));
        updateSourceMarkerVisibility();

        if (!markedPart) {
            targetsField = null;
            modifierRows = null;
            session.invalidate(PadSession.INSPECTOR);
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

    /** Apply one committed source-field value to the document. */
    private void commitSourceEdit(EditText source, Editable s) {
        if (editingModifier || modifierBodyEnd > editor.length() || source != sourceField) return;
        editingModifier = true;
        try {
            int at = modifierBodyStart;
            String replacement = s.toString();
            if (sourceCompletionAt >= 0) {
                String candidate = DocBridge.mathCompletion(editor.getText().toString(), at + sourceCompletionAt);
                if (!candidate.equals("null")) {
                    try {
                        JSONObject completion = new JSONObject(candidate);
                        int start = completion.getInt("start") - at;
                        int end = completion.getInt("end") - at;
                        if (start >= 0 && end == sourceCompletionAt && end <= replacement.length()) {
                            String glyph = completion.getString("text");
                            replacement = replacement.substring(0, start) + glyph + replacement.substring(end);
                            int caret = start + glyph.length() + 1;
                            source.setText(replacement);
                            source.setSelection(Math.min(caret, source.length()));
                        }
                    } catch (Exception e) { Log.w("emthin-pad", "could not complete math input", e); }
                }
                sourceCompletionAt = -1;
            }
            // A marker identifier is followed by alphanumerics, so keep a
            // separator when replacing the whole body with a word; otherwise
            // #i + bold becomes the marker #ibold.
            if (modifierStart != null && !replacement.isEmpty()
                    && Character.isLetterOrDigit(replacement.codePointAt(0))) {
                replacement = " " + replacement;
            }
            String old = editor.getText().subSequence(at, modifierBodyEnd).toString();
            boolean changed = !old.contentEquals(replacement);
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
            if (changed) {
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
            }
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
        if (modifierRefreshDeferred) {
            modifierRefreshDeferred = false;
            updateModifierPanel();
        }
    }

    private void closeInspector() {
        finishActiveComposition();
        if (targetsField != null) commitTargetPairs(targetsField, false);
        targetsField = null;
        inspectorOpen = false;
        wholeDocumentEditing = false;
        modifierStart = modifierEnd = null;
        sourceField = null;
        mathHint = null;
        inspectorSignature = "";
        session.invalidate(PadSession.INSPECTOR);
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
        // Invalidate before the debounce expires, so an already-running
        // comparison cannot publish into a changed target.
        session.invalidate(PadSession.INSPECTOR);
        main.removeCallbacks(updateModifierComparisons);
        if (modifierStart != null && modifierEnd != null) main.postDelayed(updateModifierComparisons, 300);
    }

    private void requestModifierComparisons() {
        if (handle == 0 || modifierStart == null || modifierEnd == null || modifierRows == null) return;
        final PadSession.Owner owner = session.capture(PadSession.INSPECTOR,
                modifierStart + "\u0000" + modifierEnd);
        final String text = editor.getText().toString();
        final String snapshot = DocBridge.snapshot(handle);
        final String start = modifierStart, end = modifierEnd;
        final float scale = Math.max(1f, Math.min(3f, getResources().getDisplayMetrics().density));
        final float width = Math.max(60f, modifierRows.getWidth() / (2f * scale));
        renderWork.submit("inspector", RenderController.EXPLICIT, new RenderController.Job() {
            @Override public void run() {
                final String result = DocBridge.modifierComparisons(text, snapshot, start, end, width, scale);
                main.post(new Runnable() {
                    @Override public void run() {
                        if (session.isLatest(owner) && modifierRows != null) {
                            showModifierComparisons(result);
                        }
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
        session.invalidate(PadSession.RENDER);
        pagesStale = true;
        renderPending = true;
        main.removeCallbacks(renderNow);
        main.postDelayed(renderNow, 300);
    }

    /**
     * Layout input captured when a render starts: reading width, marker
     * visibility and explicit folds. The caret and search/selection overlays
     * are deliberately excluded — they repaint pages instead of relayouting.
     * Source content and marks are captured separately through snapshot bytes.
     */
    private String renderPresentationKey() {
        int pageWidth = basePageWidth();
        StringBuilder key = new StringBuilder();
        key.append(readingSizePt).append('|').append(pageWidth).append('|')
                .append(showMarkers ? 1 : 0);
        for (String[] pair : expandedSegments.values()) {
            key.append('|').append(pair[0]).append('\u0000').append(pair[1]);
        }
        return key.toString();
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
        main.removeCallbacks(touchLongPress);
        touchSelecting = false;
        touchSelectionPage = null;
        visibleScrollY = -1;
        long captureStart = System.nanoTime();
        renderSnapshot = DocBridge.snapshotBytes(handle);
        trace("renderCost", "captureMs", (System.nanoTime() - captureStart) / 1_000_000.0);
        if (restoreScrollY >= 0 && restoreScrollWidth > 0 && previewPages.getWidth() > 0) {
            restoreScrollY = Math.round((float) restoreScrollY * previewPages.getWidth()
                    / restoreScrollWidth);
            restoreScrollWidth = 0;
        }
        renderOwner = session.capture(PadSession.RENDER, renderPresentationKey());
        renderLayoutId = renderOwner.documentId + ":" + renderOwner.instanceId + ":"
                + renderOwner.mutationSeq + ":" + renderOwner.requestId + ":"
                + renderOwner.presentationKey;
        pagesLoaded = 0;
        totalPages = 0;
        chunkInFlight = false;
        pagesStale = true;
        // Dropping the pages clamps the ScrollView to 0. Keep the reading
        // position across the rebuild; the first paint has no position yet
        // and scrolls to the caption instead. A retained target means the
        // previous render was too short to reach it (page count shrank) —
        // keep the original reading position, not the clamped one, so an
        // undo that restores the pages can return to it.
        if (openAtCaption) {
            restoreScrollY = -1;
            restoreScrollWidth = 0;
        } else if (anchorNextRender) {
            restoreScrollY = -1;
            restoreScrollWidth = 0;
        } else if (restoreScrollY < 0) {
            restoreScrollY = previewScroll.getScrollY();
        }
        boolean retainTarget = !openAtCaption && !anchorNextRender
                && restoreScrollY >= 0;
        anchorNextRender = false;
        scrollToFirstCaption = openAtCaption && readingAnchorStable == null;
        scrollToReadingAnchorAfterChunk = readingAnchorStable != null && !retainTarget;
        trace("scrollIntent", "openAtCaption", openAtCaption,
                "restore", restoreScrollY, "retain", retainTarget,
                "reading", scrollToReadingAnchorAfterChunk,
                "caption", scrollToFirstCaption);
        // Keep the current pages visible until the replacement chunk is
        // accepted; their hit positions stay disabled meanwhile.
        trace("startRender", "owner", renderOwner, "textLen", text.length(),
                "caret", caret,
                "head", text.substring(0, Math.min(24, text.length())));
        initialPagePending = false;
        initialLocatePending = false;
        requestChunk(text, 0, caret, "initial");
    }

    /** One worker-decoded page; no bitmap work runs on the UI thread. */
    private static final class DecodedPage {
        final int index, width, height, visiblePixels;
        final double textTop, widthPt, heightPt;
        final boolean hasFigures;
        final JSONArray hits;
        final Bitmap bitmap;

        DecodedPage(int index, int width, int height, int visiblePixels, double textTop,
                    double widthPt, double heightPt, boolean hasFigures,
                    JSONArray hits, Bitmap bitmap) {
            this.index = index;
            this.width = width;
            this.height = height;
            this.visiblePixels = visiblePixels;
            this.textTop = textTop;
            this.widthPt = widthPt;
            this.heightPt = heightPt;
            this.hasFigures = hasFigures;
            this.hits = hits;
            this.bitmap = bitmap;
        }
    }

    private static final class DecodedChunk {
        final String layoutId, error, intent;
        final int total, start, captionStart;
        final List<DecodedPage> pages;
        final JSONArray pageSizes, warnings;

        DecodedChunk(String layoutId, String error, int total, int start, int captionStart,
                     List<DecodedPage> pages, JSONArray pageSizes, JSONArray warnings,
                     String intent) {
            this.layoutId = layoutId;
            this.error = error;
            this.total = total;
            this.start = start;
            this.captionStart = captionStart;
            this.pages = pages;
            this.pageSizes = pageSizes;
            this.warnings = warnings;
            this.intent = intent;
        }
    }

    /**
     * Renders pages {@code start..start + CHUNK} on the worker thread:
     * prepare/reuse the retained layout, rasterize each page, and decode and
     * sample the PNG before posting. Results are published only if no newer
     * edit superseded them. At most one chunk is in flight, so chunks append
     * in order.
     */
    private void requestChunk(final String text, final int start, final int caret,
            final String intent) {
        if (renderPending || chunkInFlight || renderOwner == null
                || (totalPages > 0 && start >= totalPages)) {
            return; // one in flight already, or nothing left to load
        }
        final PadSession.Owner owner = renderOwner;
        final byte[] snapshot = renderSnapshot;
        final String layoutId = renderLayoutId;
        JSONArray expanded = new JSONArray();
        for (String[] pair : expandedSegments.values()) expanded.put(new JSONArray(java.util.Arrays.asList(pair)));
        final String expandedJson = expanded.toString();
        final int markerVisibility = showMarkers ? 1 : 0;
        chunkInFlight = true;
        // Rasterize to the actual displayed page width, not an image that
        // must be enlarged after rasterization. Before layout, the page
        // fills the display apart from the root's horizontal padding.
        final int pageWidth = basePageWidth();
        final int displayWidth = Math.round(pageWidth * viewZoom);

        renderWork.submit("page", RenderController.VISIBLE_PAGE, new RenderController.Job() {
            @Override
            public void run() {
                // Do not spend time rasterising queued, superseded work.
                if (!session.isLatest(owner)) {
                    return;
                }
                final DecodedChunk chunk = renderAndDecodeChunk(
                        snapshot, text, layoutId, start, displayWidth,
                        markerVisibility, expandedJson, intent);
                main.post(new Runnable() {
                    @Override
                    public void run() {
                        if (session.isLatest(owner)) {
                            chunkInFlight = false;
                            applyChunk(chunk);
                        }
                    }
                });
            }
        });
    }

    /** All expensive page work, including PNG decode and pixel sampling. */
    private DecodedChunk renderAndDecodeChunk(byte[] snapshot, String text,
            String layoutId, int start, int displayWidth,
            int markerVisibility, String expandedJson, String intent) {
        long layoutStart = System.nanoTime();
        String preparedJson = DocBridge.prepareLayout(
                snapshot, text, layoutId, markerVisibility, expandedJson, readingSizePt);
        long layoutMs = (System.nanoTime() - layoutStart) / 1_000_000;
        try {
            JSONObject prepared = new JSONObject(preparedJson);
            String error = prepared.optString("error", "");
            if (!error.isEmpty()) {
                return new DecodedChunk(layoutId, error, 0, start, -1,
                        new ArrayList<DecodedPage>(), new JSONArray(), new JSONArray(), intent);
            }
            int total = prepared.getInt("total");
            int caption = prepared.optInt("captionStart", -1);
            JSONArray pageSizes = prepared.optJSONArray("pages");
            if (pageSizes == null) pageSizes = new JSONArray();
            JSONArray warnings = prepared.optJSONArray("warnings");
            if (warnings == null) warnings = new JSONArray();
            if (start >= total) {
                return new DecodedChunk(layoutId, "Page is outside the layout", total, start,
                        caption, new ArrayList<DecodedPage>(), pageSizes, warnings, intent);
            }
            int page = start;
            JSONObject pageSize = pageSizes.optJSONObject(page);
            double widthPt = pageSize == null ? PAGE_WIDTH_PT : pageSize.optDouble("widthPt", PAGE_WIDTH_PT);
            double heightPt = pageSize == null ? 841.89 : pageSize.optDouble("heightPt", 841.89);
            float scale = (float) (displayWidth / Math.max(1.0, widthPt));
            long rasterStart = System.nanoTime();
            String pageJson = DocBridge.renderLayoutPage(
                    layoutId, text, page, scale, workingPagePixels,
                    Math.max(0, page - 1), Math.min(total - 1, page + 1));
            long rasterMs = (System.nanoTime() - rasterStart) / 1_000_000;
            long decodeStart = System.nanoTime();
            long decodeMs = 0;
            JSONObject p = new JSONObject(pageJson);
            String pageError = p.optString("error", "");
            if (!pageError.isEmpty()) {
                return new DecodedChunk(layoutId, pageError, total, page, caption,
                        new ArrayList<DecodedPage>(), pageSizes, warnings, intent);
            }
            int w = p.getInt("width");
            int h = p.getInt("height");
            JSONArray hits = p.getJSONArray("hits");
            byte[] png = Base64.decode(p.getString("png"), Base64.DEFAULT);
            Bitmap bmp = BitmapFactory.decodeByteArray(png, 0, png.length);
            if (bmp == null || bmp.getWidth() != w || bmp.getHeight() != h) {
                if (bmp != null) bmp.recycle();
                throw new IllegalStateException("bad raster " + w + "x" + h);
            }
            int visible = 0;
            int[] row = new int[w];
            for (int yy = 0; yy < h; yy += 4) {
                bmp.getPixels(row, 0, w, 0, yy, w, 1);
                for (int px : row) { if ((px >>> 24) != 0) visible++; }
            }
            List<DecodedPage> pages = new ArrayList<>();
            pages.add(new DecodedPage(page, w, h, visible * 4,
                    p.optDouble("textTop", -1),
                    p.optDouble("widthPt", widthPt), p.optDouble("heightPt", heightPt),
                    p.optBoolean("hasFigures", false), hits, bmp));
            decodeMs += (System.nanoTime() - decodeStart) / 1_000_000;
            trace("renderCost", "layoutMs", layoutMs, "rasterMs", rasterMs,
                    "decodeMs", decodeMs, "pages", pages.size(), "layoutId", layoutId);
            return new DecodedChunk(layoutId, "", total, page, caption,
                    pages, pageSizes, warnings, intent);
        } catch (Exception e) {
            return new DecodedChunk(layoutId, "render: " + e.getMessage(), 0, start, -1,
                    new ArrayList<DecodedPage>(), new JSONArray(), new JSONArray(), intent);
        }
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

    /** Applies one worker-decoded chunk to the page stack. */
    private void applyChunk(DecodedChunk chunk) {
        long applyStart = System.nanoTime();
        try {
            if (!chunk.error.isEmpty()) {
                showPreviewError(chunk.error);
                return;
            }
            totalPages = chunk.total;
            if (!chunk.layoutId.equals(publishedLayoutId)
                    || renderedPages.size() != totalPages) {
                publishedLayoutId = chunk.layoutId;
                resetPageStack(chunk.total, chunk.pageSizes);
            }
            pagesStale = false;
            captionStart = chunk.captionStart;
            List<DecodedPage> pages = chunk.pages;
            trace("applyChunk", "total", totalPages, "pages", pages.size(),
                    "start", chunk.start);
            for (DecodedPage p : pages) applyDecodedPage(p);
            if (!initialPagePending) {
                initialPagePending = true;
                locateInitialPage(editor.getSelectionStart());
            } else if (!chunk.intent.isEmpty()) {
                finishPageIntent(chunk.start, chunk.intent);
            }
            trimPageBitmaps();
            ensureVisiblePages();
            restorePreviewScroll();
            if (searchBar.getVisibility() == View.VISIBLE && searchCurrent >= 0) scrollSearchMatch();
            JSONArray warnings = chunk.warnings;
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
        } finally {
            trace("renderCost", "uiApplyMs", (System.nanoTime() - applyStart) / 1_000_000.0,
                    "start", chunk.start, "pages", chunk.pages.size());
        }
    }

    /** Create one full-height placeholder per page, then materialize on demand. */
    private void resetPageStack(int total, JSONArray sizes) {
        releasePageBitmaps();
        totalPages = total;
        int width = Math.round(basePageWidth() * viewZoom);
        for (int i = 0; i < total; i++) {
            JSONObject size = sizes.optJSONObject(i);
            double widthPt = size == null ? PAGE_WIDTH_PT : size.optDouble("widthPt", PAGE_WIDTH_PT);
            double heightPt = size == null ? 841.89 : size.optDouble("heightPt", 841.89);

            TextView caption = new TextView(this);
            caption.setTextColor(Color.parseColor("#6E7A87"));
            caption.setTextSize(10f);
            caption.setPadding(0, dp(PAGE_GAP_DP), 0, dp(2));
            caption.setText("page " + (i + 1) + " / " + total);
            previewPages.addView(caption);

            RenderedPage page = new RenderedPage(this, i);
            page.caption = caption;
            page.widthPt = widthPt;
            page.heightPt = heightPt;
            page.setBackgroundColor(Color.parseColor("#14181D"));
            page.setClickable(true);
            page.setContentDescription("Page " + (i + 1) + " of " + total);
            installPageTouch(page);
            int heightPx = Math.max(1,
                    Math.round(width * (float) (heightPt / Math.max(1.0, widthPt))));
            previewPages.addView(page, new LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.MATCH_PARENT, heightPx));
            renderedPages.add(page);
        }
        initialPagePending = false;
        initialLocatePending = false;
        pagesLoaded = 0;
        retainedBitmapBytes = 0;
        retainedPages.clear();
        trace("pageStack", "total", total, "bitmapBudget", pageBitmapBudget,
                "workingPixels", workingPagePixels);
    }

    private void installPageTouch(final RenderedPage page) {
        page.setOnTouchListener(new View.OnTouchListener() {
            private final int slop = android.view.ViewConfiguration.get(MainActivity.this)
                    .getScaledTouchSlop();
            private boolean moved;
            @Override public boolean onTouch(View view, MotionEvent event) {
                switch (event.getActionMasked()) {
                    case MotionEvent.ACTION_DOWN:
                        if (pagesStale) return true;
                        touchSelectionPage = page;
                        touchSelectionAnchor = selectionAnchor;
                        touchDownX = event.getX();
                        touchDownY = event.getY();
                        moved = false;
                        touchSelecting = false;
                        main.removeCallbacks(touchLongPress);
                        if (!pagesStale && page.loaded) {
                            main.postDelayed(touchLongPress,
                                    android.view.ViewConfiguration.getLongPressTimeout());
                        }
                        return true;
                    case MotionEvent.ACTION_MOVE:
                        if (!moved
                                && (Math.abs(event.getX() - touchDownX) > slop
                                    || Math.abs(event.getY() - touchDownY) > slop)) {
                            moved = true;
                            main.removeCallbacks(touchLongPress);
                        }
                        if (touchSelecting && !pagesStale) {
                            int offset = caretOffsetAtPoint(page, event.getX(), event.getY());
                            if (offset >= 0) {
                                setDocumentCaret(offset);
                                page.invalidate();
                                showSelectionBar();
                            }
                        }
                        return true;
                    case MotionEvent.ACTION_UP:
                        main.removeCallbacks(touchLongPress);
                        if (touchSelecting) {
                            int offset = caretOffsetAtPoint(page, event.getX(), event.getY());
                            if (offset >= 0) setDocumentCaret(offset);
                            showSelectionBar();
                        } else if (!moved && !pagesStale) {
                            selectPageGlyph(page, event.getX(), event.getY());
                        }
                        touchSelecting = false;
                        touchSelectionPage = null;
                        return true;
                    case MotionEvent.ACTION_CANCEL:
                        main.removeCallbacks(touchLongPress);
                        if (!touchSelecting) {
                            touchSelectionPage = null;
                        }
                        touchSelecting = false;
                        return true;
                    default:
                        return true;
                }
            }
        });
    }

    private int caretOffsetAtPoint(RenderedPage page, float x, float y) {
        if (page == null || !page.loaded || page.getWidth() <= 0) return -1;
        float scale = page.pointScale();
        float px = x / Math.max(0.0001f, scale);
        float py = y / Math.max(0.0001f, scale);
        int bestOffset = -1;
        double bestDistance = Double.MAX_VALUE;
        for (int i = 0; i < page.hits.length(); i++) {
            JSONArray hit = page.hits.optJSONArray(i);
            if (hit == null) continue;
            float left = (float) hit.optDouble(0);
            float right = (float) hit.optDouble(2);
            float top = (float) hit.optDouble(1);
            float bottom = (float) hit.optDouble(3);
            float dx = px < left ? left - px : px > right ? px - right : 0f;
            float dy = py < top ? top - py : py > bottom ? py - bottom : 0f;
            double distance = dy * 1000.0 + dx;
            if (distance < bestDistance) {
                bestDistance = distance;
                int offset = hit.optInt(4, -1);
                if (offset >= 0) {
                    int after = snapGrapheme(offset, 1);
                    bestOffset = px > (left + right) / 2f ? after : offset;
                }
            }
        }
        return bestOffset < 0 ? -1 : snapGrapheme(bestOffset, 0);
    }

    private void selectWordAt(int offset) {
        String text = editor.getText().toString();
        int start = DocBridge.wordBoundary(text, Math.max(0, Math.min(text.length(), offset)), -1);
        int end = DocBridge.wordBoundary(text, Math.max(0, Math.min(text.length(), offset)), 1);
        if (start == end) {
            start = snapGrapheme(offset, 0);
            end = snapGrapheme(offset, 1);
        }
        if (start == end) return;
        setSelectionAnchor(end);
        setDocumentCaret(start);
        touchSelectionAnchor = start;
        if (touchSelectionPage != null) touchSelectionPage.invalidate();
    }

    private void showSelectionBar() {
        if (selectionBar == null || !hasRenderedSelection()) return;
        selectionBar.setVisibility(View.VISIBLE);
    }

    private void hideSelectionBar() {
        if (selectionBar != null) selectionBar.setVisibility(View.GONE);
    }


    private void selectPageGlyph(RenderedPage page, float x, float y) {
        if (page.getWidth() <= 0) return;
        float scale = (float) (page.widthPt / page.getWidth());
        float px = x * scale;
        float py = y * scale;
        try {
            for (int i = 0; i < page.hits.length(); i++) {
                JSONArray h = page.hits.getJSONArray(i);
                float left = (float) h.getDouble(0);
                float right = (float) h.getDouble(2);
                if (px >= Math.min(left, right) && px <= Math.max(left, right)
                        && py >= h.getDouble(1) && py <= h.getDouble(3)) {
                    int caret = snapGrapheme(h.getInt(4), 0);
                    int citation = citationAt(caret);
                    if (citation > 0) { showCitations(citation); return; }
                    if (toggleFoldAt(caret, true)) return;
                    clearSelectionAnchor();
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
        if (page.hits.length() == 0) {
            inspectorOpen = true;
            wholeDocumentEditing = false;
            inspectorSignature = "";
            updateModifierPanel();
            focusSourceCaret();
        }
    }

    private void applyDecodedPage(DecodedPage p) {
        if (p.index < 0 || p.index >= renderedPages.size()) return;
        RenderedPage page = renderedPages.get(p.index);
        Bitmap old = page.getDrawable() instanceof BitmapDrawable
                ? ((BitmapDrawable) page.getDrawable()).getBitmap() : null;
        if (old != null && old != p.bitmap && !old.isRecycled()) {
            retainedBitmapBytes = Math.max(0, retainedBitmapBytes - page.bitmapBytes);
            old.recycle();
        }
        page.setImageBitmap(p.bitmap);
        page.hits = p.hits;
        page.textTop = p.textTop;
        page.widthPt = p.widthPt;
        page.heightPt = p.heightPt;
        page.hasFigures = p.hasFigures;
        page.bitmapBytes = p.bitmap.getAllocationByteCount();
        if (!page.loaded) {
            page.loaded = true;
            pagesLoaded++;
        }
        retainedBitmapBytes += page.bitmapBytes;
        if (!retainedPages.contains(page)) retainedPages.add(page);
        int width = Math.round(basePageWidth() * viewZoom);
        int height = Math.max(1, Math.round(width
                * (float) (page.heightPt / Math.max(1.0, page.widthPt))));
        page.setLayoutParams(new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, height));
        if (page.caption != null) page.caption.setText("page " + (p.index + 1) + " / " + totalPages);
        trace("pageRaster", "page", p.index, "wh", p.width + "x" + p.height,
                "visiblePx~", p.visiblePixels, "bytes", page.bitmapBytes);
        trace("pageLayout", "page", p.index, "raster", p.width + "x" + p.height,
                "pw", width, "ih", height, "textTop", p.textTop);
        if (!inspectorOpen && !(getCurrentFocus() instanceof EditText)) focusRenderedCaret();
    }

    private void locateInitialPage(int caret) {
        int offset = caret;
        String intent = "caret";
        if (scrollToReadingAnchorAfterChunk && readingAnchorStable != null) {
            int resolved = readingAnchorStable.resolve(handle, session.documentId());
            if (resolved >= 0) offset = resolved;
            else if (readingAnchorFallback >= 0) offset = readingAnchorFallback;
            intent = "reading";
        } else if (scrollToFirstCaption) {
            scrollToFirstCaption = false;
            if (captionStart >= 0) {
                offset = captionStart;
                intent = "caption";
            }
        }
        initialLocatePending = true;
        trace("locateInitial", "offset", offset, "intent", intent,
                "restore", restoreScrollY);
        locatePage(offset, intent);
    }

    private void locatePage(final int offset, final String intent) {
        final PadSession.Owner owner = renderOwner;
        final String text = editor.getText().toString();
        final String layoutId = renderLayoutId;
        renderWork.submit("locate", RenderController.VISIBLE_PAGE, new RenderController.Job() {
            @Override public void run() {
                if (!session.isLatest(owner)) return;
                int page = 0;
                try {
                    JSONObject located = new JSONObject(DocBridge.locateLayoutRange(
                            layoutId, text, Math.max(0, offset), Math.max(0, offset)));
                    page = located.optInt("page", 0);
                } catch (Exception ignored) { }
                final int target = page;
                main.post(new Runnable() {
                    @Override public void run() {
                        if (!session.isLatest(owner)) return;
                        requestPage(target, intent);
                    }
                });
            }
        });
    }

    private void requestPage(int page, String intent) {
        initialLocatePending = false;
        if (page < 0 || page >= renderedPages.size()) return;
        RenderedPage target = renderedPages.get(page);
        if (target.loaded) {
            finishPageIntent(page, intent);
            ensureVisiblePages();
            return;
        }
        requestChunk(editor.getText().toString(), page, editor.getSelectionStart(), intent);
    }

    private void finishPageIntent(int page, String intent) {
        if (page < 0 || page >= renderedPages.size()) return;
        RenderedPage target = renderedPages.get(page);
        if ("reading".equals(intent)) {
            if (scrollToReadingAnchor()) {
                scrollToReadingAnchorAfterChunk = false;
            }
        } else if ("caption".equals(intent) && target.textTop >= 0) {
            int width = Math.max(1, target.getWidth());
            int offset = Math.round((float) target.textTop * width
                    / Math.max(1.0f, (float) target.widthPt));
            int y = Math.max(0, target.getTop() + offset - dp(4));
            previewScroll.scrollTo(0, y);
            trace("captionScroll", "page", page + 1, "topPt", target.textTop,
                    "px", offset, "scrollY", y);
        } else if ("caret".equals(intent) || "search".equals(intent)) {
            focusRenderedCaret();
        }
        ensureVisiblePages();
    }

    private int[] visiblePageRange() {
        if (renderedPages.isEmpty()) return new int[] {0, 0};
        int top = previewScroll.getScrollY();
        int bottom = top + Math.max(1, previewScroll.getHeight());
        int first = -1, last = -1;
        for (int i = 0; i < renderedPages.size(); i++) {
            RenderedPage page = renderedPages.get(i);
            int pageTop = page.getTop();
            int pageBottom = pageTop + page.getHeight();
            if (pageBottom >= top && pageTop <= bottom) {
                if (first < 0) first = i;
                last = i;
            }
        }
        if (first < 0) {
            first = last = Math.max(0, Math.min(renderedPages.size() - 1,
                    previewScroll.getScrollY() / Math.max(1, dp(400))));
        }
        return new int[] {first, last};
    }

    /** Request only pages in the visible window plus one neighbor each way. */
    private void ensureVisiblePages() {
        if (initialLocatePending || chunkInFlight || renderPending || pagesStale
                || renderedPages.isEmpty() || renderOwner == null) return;
        int[] range = visiblePageRange();
        int from = Math.max(0, range[0] - 1);
        int to = Math.min(renderedPages.size() - 1, range[1] + 1);
        int wanted = -1;
        for (int i = range[0]; i <= range[1]; i++) {
            if (!renderedPages.get(i).loaded) { wanted = i; break; }
        }
        if (wanted < 0 && from < range[0] && !renderedPages.get(from).loaded) wanted = from;
        if (wanted < 0 && to > range[1] && !renderedPages.get(to).loaded) wanted = to;
        if (wanted >= 0) {
            requestChunk(editor.getText().toString(), wanted,
                    editor.getSelectionStart(), "visible");
        }
    }

    /** Recycle bitmaps outside the visible neighborhood and enforce the budget. */
    private void trimPageBitmaps() {
        int[] range = visiblePageRange();
        int from = Math.max(0, range[0] - 1);
        int to = Math.min(renderedPages.size() - 1, range[1] + 1);
        long now = System.currentTimeMillis();
        for (int i = from; i <= to; i++) renderedPages.get(i).lastVisible = now;
        for (int i = 0; i < renderedPages.size(); i++) {
            if (i >= from && i <= to) continue;
            releasePageBitmap(renderedPages.get(i));
        }
        while (retainedBitmapBytes > pageBitmapBudget) {
            RenderedPage oldest = null;
            for (RenderedPage page : retainedPages) {
                if (!page.loaded || (page.index >= from && page.index <= to)) continue;
                if (oldest == null || page.lastVisible < oldest.lastVisible) oldest = page;
            }
            if (oldest == null) break;
            releasePageBitmap(oldest);
        }
    }

    private void releasePageBitmap(RenderedPage page) {
        Drawable drawable = page.getDrawable();
        page.setImageDrawable(null);
        if (drawable instanceof BitmapDrawable) {
            Bitmap bitmap = ((BitmapDrawable) drawable).getBitmap();
            if (bitmap != null && !bitmap.isRecycled()) bitmap.recycle();
        }
        retainedBitmapBytes = Math.max(0, retainedBitmapBytes - page.bitmapBytes);
        page.bitmapBytes = 0;
        page.hits = new JSONArray();
        page.loaded = false;
        retainedPages.remove(page);
        pagesLoaded = Math.max(0, pagesLoaded - 1);
        trace("pageEvict", "page", page.index, "bytes", retainedBitmapBytes);
    }

    /**
     * Drops the old page rasters before the rebuild. removeAllViews()
     * alone leaves the bitmaps for a later GC; on the pad's large pages
     * that is roughly 25 MB of Java heap held while the new render (and
     * its own base64 payload) is in flight.
     */
    private void releasePageBitmaps() {
        renderedPages.clear();
        retainedPages.clear();
        retainedBitmapBytes = 0;
        pagesLoaded = 0;
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
        final PadSession.Owner owner = renderOwner;
        previewScroll.post(new Runnable() {
            @Override
            public void run() {
                // A touch, a newer render or destruction may have superseded
                // this target between the post and this runnable.
                if (renderOwner != owner || !session.isLatest(owner)
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
                trace("restoreScroll", "owner", owner, "y", target, "actual", actual);
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
            // One accepted state transition: text, undo/redo, a mark change or
            // a modifier application. Marks-only edits bump this even when
            // the text revision is unchanged.
            session.recordMutation();
            refreshSelectionAnchor();
            refreshReadingAnchor();
            if (!restoringState) unsaved = true;

            String text = o.getString("text");
            modelText = text;
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
                + " segs · " + props + extra + saveStateLabel());
    }

    private String saveStateLabel() {
        if (!recoveryReady) return "";
        return unsaved ? " · unsaved" : " · saved";
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
