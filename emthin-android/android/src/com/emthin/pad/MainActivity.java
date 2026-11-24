package com.emthin.pad;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.DialogInterface;
import android.content.SharedPreferences;
import android.content.Intent;
import android.content.res.Configuration;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.graphics.Color;
import android.graphics.Canvas;
import android.graphics.Paint;
import android.graphics.drawable.GradientDrawable;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.net.Uri;
import android.opengl.EGL14;
import android.opengl.EGLConfig;
import android.opengl.EGLContext;
import android.opengl.EGLDisplay;
import android.opengl.EGLSurface;
import android.opengl.GLES20;
import android.opengl.GLUtils;
import android.text.Editable;
import android.text.InputType;
import android.text.Spannable;
import android.text.SpannableStringBuilder;
import android.text.TextWatcher;
import android.text.style.BackgroundColorSpan;
import android.text.style.ReplacementSpan;
import android.util.Base64;
import android.util.Log;
import android.view.MotionEvent;
import android.view.KeyEvent;
import android.view.Gravity;
import android.view.SubMenu;
import android.view.Surface;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.View;
import android.view.ViewTreeObserver;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputConnectionWrapper;
import android.widget.AdapterView;
import android.widget.ArrayAdapter;
import android.widget.HorizontalScrollView;
import android.widget.EditText;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.PopupMenu;
import android.widget.ScrollView;
import android.widget.FrameLayout;
import android.widget.Spinner;
import android.widget.TextView;

import org.json.JSONArray;
import org.json.JSONObject;

import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
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
    private static final String PREF_GATEWAY_ENDPOINTS = "gatewayEndpoints";

    /** Translucent highlight over ranges the model resolved to segments. */
    private static final int SEGMENT_HIGHLIGHT = 0x3346B883;

    /** Pages per render request — bounded so one JNI response stays small. */
    private static final int CHUNK = 3;
    /** Nominal A4 page width in points — Typst's default page size. */
    private static final float PAGE_WIDTH_PT = 595.28f;
    /** Vertical gap between page frames (dp). */
    private static final int PAGE_GAP_DP = 12;

    private long handle;
    /** Native document-sharing worker handle (K6). Owned by this activity. */
    private long shareHandle;
    /** A watching peer can copy content but cannot commit document edits. */
    private boolean shareWatchOnly;
    /** Guards the publish -> drain -> apply -> save recursion. */
    private boolean publishingShare;
    private EditText editor;
    private TextView undoButton;
    private boolean canRedo;
    private EditText targetsField;
    private String appliedTargetList = "";
    private boolean showMarkers;
    private TextView status;
    private LinearLayout searchBar;
    private EditText searchField;
    private TextView searchCount;
    private TextView findScopeButton;
    private LinearLayout replaceRow;
    private LinearLayout findResults;
    private ScrollView findResultsScroll;
    private EditText replaceField;
    private TextView replaceStatus;
    private TextView replaceAllButton;
    private FindController find;
    private boolean replaceVisible;
    private FindController.Page replacePlan;
    private String replacePlanText = "";
    private SearchOrigin searchOriginState;
    private String findResultsSignature = "";
    private boolean findScrollPending;
    /** Selection to restore when the next Undo reverses a replace batch. */
    private int replaceUndoStart = -1;
    private int replaceUndoEnd = -1;
    private int selectionAnchor = -1;
    private StableAnchor selectionAnchorStable;
    /** Touch selection state; the long-press timeout may be pending. */
    private LinearLayout selectionBar;
    private RenderedPage touchSelectionPage;
    private int touchSelectionAnchor = -1;
    private int touchSelectionHandle = -1;
    private float touchDownX, touchDownY;
    private boolean touchSelecting;
    /** Source offset of the virtual accessibility text block with focus. */
    private int accessibilityFocusOffset = -1;
    private final Runnable touchLongPress = new Runnable() {
        @Override public void run() {
            if (touchSelectionPage == null || !touchSelectionPage.loaded || pagesStale) return;
            int terminal = touchSelectionPage.terminalIndexAt(touchDownX, touchDownY);
            if (terminal >= 0) {
                if (terminalMousePressed && terminalMousePage == touchSelectionPage) {
                    int[] cell = touchSelectionPage.terminalCellAt(terminalMouseIndex,
                            touchDownX, touchDownY);
                    if (cell != null) {
                        sendTerminalMouse(touchSelectionPage, terminalMouseIndex,
                                cell[0], cell[1], 0, false, 0);
                    }
                    terminalMousePressed = false;
                    terminalMousePage = null;
                    terminalMouseIndex = -1;
                }
                selectTerminalWord(touchSelectionPage, terminal, touchDownX, touchDownY);
                if (terminalSelectionPage == null) return;
                touchSelectionPage.performHapticFeedback(
                        android.view.HapticFeedbackConstants.LONG_PRESS);
                showTerminalSelectionBar();
                return;
            }
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
    private static final int REQUEST_PROOF = 45;
    private static final int REQUEST_QR = 46;
    private Uri documentUri;
    /** Human name of the open source file; survives a source-only Open. */
    private String documentLabel;
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
    /** Find scans are literal and bounded; they never use the render worker. */
    private final ExecutorService findWorker = Executors.newSingleThreadExecutor();
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
    /** One caption and canvas per rendered page, stacked. */
    private LinearLayout previewPages;
    private LinearLayout previewBox;
    private ScrollView previewScroll;
    private HorizontalScrollView previewHScroll;
    private TextView previewError;
    private LinearLayout modifierPanel;
    private LinearLayout modifierRows;
    private LinearLayout modifierSuggestions;
    private LinearLayout expandedModifierRow;
    private int modifierRowGeneration;
    private final InspectorView.ComparisonCache modifierRowCache =
            new InspectorView.ComparisonCache();
    private EditText sourceField;
    private TextView mathHint;
    private boolean syncingCaret;
    private final java.util.ArrayList<RenderedPage> renderedPages = new java.util.ArrayList<>();
    private JSONArray documentMarkers = new JSONArray();
    private JSONArray documentModifierLinks = new JSONArray();
    private JSONArray documentCitations = new JSONArray();
    private JSONArray documentSegments = new JSONArray();
    /** Empty until the first `sessions.list` answer; never a scripted list. */
    private SessionCatalog sessionCatalog = SessionCatalog.empty();
    /** Set while the Sessions picker is open so a refresh can re-render it. */
    private LinearLayout sessionsRows;
    /** The open pairing dialog, so the scanner can hand back to it. */
    private AlertDialog pairingDialog;
    private boolean sessionRefreshInFlight;
    private final CitationController citationController = new CitationController();
    private final java.util.LinkedHashMap<String, String[]> expandedSegments = new java.util.LinkedHashMap<>();
    private final NavigationController navigation = new NavigationController();
    private static final int NAV_SECTIONS = 0, NAV_DEFINITIONS = 1, NAV_REFERENCES = 2;
    private int lastNavigateTab = NAV_REFERENCES;
    private JSONObject documentSemantics = new JSONObject();
    private JSONArray currentDiagnostics = new JSONArray();
    private String statusText = "";
    private int diagnosticCount;
    private boolean deviceTest;
    private String fixtureText, fixtureModifier, fixtureTargets, fixtureModifier2, fixtureTargets2, fixtureMarker;
    private String fixtureOpenPath;
    private String fixtureGateway;
    private String fixtureGatewayGeneration;
    private String fixtureGatewayCa;
    /** Lane-only origin for pairing HTTP; the link still parses as a tailnet one. */
    private String fixturePairBase;
    /** Lane-only origin for a paired row's socket, keeping its Core ID. */
    private String fixtureConnect;
    private GatewayLeaseClient gatewayLease;
    /** Paired gateways, revalidated on every load; see {@link GatewayEndpoints}. */
    private GatewayEndpoints gatewayEndpoints;
    private boolean pairingInFlight;
    private String pairingStatusText = "No gateway paired";
    private LinearLayout pairingRows;
    private boolean gatewayReady;
    private boolean gatewayConnected;
    private boolean gatewayAttaching;
    /** The figure reference this lease serves, so a failure can re-resolve it. */
    private String gatewayLeaseReference;
    /** One registry retry per lease; a second failure is reported, not looped. */
    private boolean gatewayLeaseRetried;
    private boolean sessionPinFixturePending;
    private boolean figureA11yFixturePending;
    private boolean vtSceneFixturePending;
    private boolean vtSceneFixtureRunning;
    private boolean vtSceneZoomFixture;
    private boolean vtSceneA11yFixture;
    private boolean vtScenePagesFixture;
    private int vtSceneFrame;
    private String terminalComposing = "";
    private LinearLayout terminalKeyRow;
    private LinearLayout terminalSelectionBar;
    private static final int TERMINAL_CELL_WIDTH = 10;
    private static final int TERMINAL_CELL_HEIGHT = 18;
    /** Inclusive cell bounds of a long-press/drag selection in one scene. */
    private RenderedPage terminalSelectionPage;
    private int terminalSelectionIndex = -1;
    private int terminalSelX0, terminalSelY0, terminalSelX1, terminalSelY1;
    private boolean terminalSelecting;
    /** Active button press routed to a program that enabled mouse reporting. */
    private RenderedPage terminalMousePage;
    private int terminalMouseIndex = -1;
    private boolean terminalMousePressed;
    /** Scenes that outlive a page-stack rebuild (view zoom/font reflow). */
    private final java.util.ArrayList<TerminalTransfer> pendingTerminalTransfers =
            new java.util.ArrayList<>();
    /** Document identity that owns the current page stack. */
    private String pageStackDocumentId = "";
    private static final class TerminalTransfer {
        final NativeTerminal scene;
        final String reference;
        final int pageIndex;
        final boolean focused;
        final boolean watchOnly;
        final boolean conflict;
        boolean claimed;
        TerminalTransfer(NativeTerminal scene, String reference, int pageIndex,
                boolean focused, boolean watchOnly, boolean conflict) {
            this.scene = scene;
            this.reference = reference;
            this.pageIndex = pageIndex;
            this.focused = focused;
            this.watchOnly = watchOnly;
            this.conflict = conflict;
        }
    }
    private boolean fixturePending;
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
        documentLabel = prefs.getString("documentLabel", null);
        if (documentLabel == null && documentUri != null)
            documentLabel = documentUri.getLastPathSegment();
        documentIsSnapshot = prefs.getBoolean(PREF_DOCUMENT_SNAPSHOT, true);
        String rememberedHash = prefs.getString(PREF_DOCUMENT_HASH, "");
        if (rememberedHash != null && !rememberedHash.isEmpty()) {
            fileStamp = new ProviderIo.Stamp(rememberedHash,
                    prefs.getLong(PREF_DOCUMENT_SIZE, 0),
                    prefs.getLong(PREF_DOCUMENT_MODIFIED, 0));
        }

        // Only the disposable test manifest enables launch fixtures.
        deviceTest = deviceTestEnabled();
        Intent testIntent = deviceTest ? getIntent() : new Intent();
        fixtureText = testIntent.getStringExtra("emthin.fixture");
        String fixtureB64 = testIntent.getStringExtra("emthin.fixtureB64");
        if (fixtureB64 != null) {
            try {
                fixtureText = new String(Base64.decode(fixtureB64, Base64.DEFAULT),
                        java.nio.charset.StandardCharsets.UTF_8);
            } catch (Exception e) {
                fixtureText = null;
            }
        }
        fixtureModifier = testIntent.getStringExtra("emthin.fixtureModifier");
        fixtureTargets = testIntent.getStringExtra("emthin.fixtureTargets");
        String fixtureTargetsB64 = testIntent.getStringExtra("emthin.fixtureTargetsB64");
        if (fixtureTargetsB64 != null) {
            try {
                fixtureTargets = new String(Base64.decode(fixtureTargetsB64, Base64.DEFAULT),
                        java.nio.charset.StandardCharsets.UTF_8);
            } catch (Exception e) {
                fixtureTargets = null;
            }
        }
        fixtureModifier2 = testIntent.getStringExtra("emthin.fixtureModifier2");
        fixtureTargets2 = testIntent.getStringExtra("emthin.fixtureTargets2");
        String fixtureTargets2B64 = testIntent.getStringExtra("emthin.fixtureTargets2B64");
        if (fixtureTargets2B64 != null) {
            try {
                fixtureTargets2 = new String(Base64.decode(fixtureTargets2B64, Base64.DEFAULT),
                        java.nio.charset.StandardCharsets.UTF_8);
            } catch (Exception e) {
                fixtureTargets2 = null;
            }
        }
        fixtureMarker = testIntent.getStringExtra("emthin.fixtureMarker");
        fixtureOpenPath = testIntent.getStringExtra("emthin.fixtureOpenPath");
        fixtureGateway = testIntent.getStringExtra("emthin.fixtureGateway");
        fixtureGatewayGeneration = testIntent.getStringExtra("emthin.fixtureGatewayGeneration");
        fixtureGatewayCa = testIntent.getStringExtra("emthin.fixtureGatewayCa");
        fixturePairBase = testIntent.getStringExtra("emthin.fixturePairBase");
        fixtureConnect = testIntent.getStringExtra("emthin.fixtureConnect");
        // A shell cannot write into this app's directory on the Waydroid image
        // (run-as is refused without CAP_SETGID), so the lane can hand the
        // fixture certificate over as launch data instead. Fixture-only: the
        // production path is a public CA through the platform trust store.
        String fixtureGatewayCaPem = testIntent.getStringExtra("emthin.fixtureGatewayCaPem");
        if ((fixtureGatewayCa == null || fixtureGatewayCa.trim().isEmpty())
                && fixtureGatewayCaPem != null && !fixtureGatewayCaPem.trim().isEmpty()) {
            try {
                byte[] pem = Base64.decode(fixtureGatewayCaPem.trim(), Base64.DEFAULT);
                java.io.File ca = new java.io.File(getFilesDir(), "fixture-ca.pem");
                java.io.FileOutputStream out = new java.io.FileOutputStream(ca);
                try {
                    out.write(pem);
                } finally {
                    out.close();
                }
                fixtureGatewayCa = ca.getAbsolutePath();
            } catch (Exception e) {
                trace("gatewayLease", "phase", "failed", "reason", "ca",
                        "detail", String.valueOf(e.getMessage()));
            }
        }
        gatewayEndpoints = new GatewayEndpoints(new GatewayEndpoints.Store() {
            @Override public String read() {
                return prefs().getString(PREF_GATEWAY_ENDPOINTS, "");
            }

            @Override public void write(String value) {
                prefs().edit().putString(PREF_GATEWAY_ENDPOINTS, value).apply();
            }
        });
        int paired = gatewayEndpoints.list().size();
        if (paired > 0) pairingStatusText = paired == 1 ? "1 gateway paired" : paired + " gateways paired";
        sessionPinFixturePending = testIntent.getBooleanExtra("emthin.fixtureSessionPin", false);
        figureA11yFixturePending = testIntent.getBooleanExtra("emthin.fixtureFigureA11y", false);
        vtSceneFixturePending = testIntent.getBooleanExtra("emthin.fixtureVtScene", false);
        vtSceneZoomFixture = testIntent.getBooleanExtra("emthin.fixtureVtZoom", false);
        vtSceneA11yFixture = testIntent.getBooleanExtra("emthin.fixtureVtA11y", false);
        vtScenePagesFixture = testIntent.getBooleanExtra("emthin.fixtureVtPages", false);
        float fixtureZoom = testIntent.getFloatExtra("emthin.fixtureViewZoom", -1f);
        if (fixtureZoom >= 1f) viewZoom = Math.min(4f, fixtureZoom);
        fixturePending = fixtureText != null || sessionPinFixturePending
                || figureA11yFixturePending || vtSceneFixturePending;

        buildUi();
        previewBox.post(new Runnable() {
            @Override public void run() { applyViewZoomWidth(); }
        });
        // A figure resolves against the catalogue, so a pad that already has
        // paired hosts fills it before the document is first drawn rather than
        // showing every pinned session as unavailable until the picker opens.
        if (paired > 0) refreshSessionCatalog();
        // The editor exists so the view hierarchy is complete, but it must
        // not accept an edit that recovery is still loading.
        editor.setEnabled(false);
        setStatus("");
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
        if (testIntent.getBooleanExtra("emthin.jitSmoke", false)) {
            Log.i("emthin-pad", "cranelift roundtrip: " + DocBridge.jitRoundtrip(42));
        }

        // K9 native rule artifact: the Rust entry runs the Austral module on
        // its own worker thread; call it from a Java background thread too so
        // the device proof covers both sides of the off-UI-thread boundary.
        if (testIntent.getBooleanExtra("emthin.australRuleSmoke", false)) {
            new Thread(() -> {
                try {
                    String facts = "{\"facts\":[{\"from\":\"a\",\"to\":\"b\"},"
                            + "{\"from\":\"b\",\"to\":\"a\"}]}";
                    Log.i("emthin-pad", "austral rule: " + DocBridge.australRule(facts));
                } catch (Throwable t) {
                    Log.e("emthin-pad", "austral rule failed: " + t, t);
                }
            }, "emthin-austral-rule").start();
        }
    }

    private boolean deviceTestEnabled() {
        try {
            Bundle metadata = getPackageManager().getApplicationInfo(getPackageName(),
                    android.content.pm.PackageManager.GET_META_DATA).metaData;
            return metadata != null && metadata.getBoolean("com.emthin.pad.deviceTest");
        } catch (android.content.pm.PackageManager.NameNotFoundException e) {
            return false;
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

        if (deviceTest && getIntent().getBooleanExtra("emthin.historySmoke", false)) {
            // The callback runs after setup on the model owner. Even read-only
            // capture cannot borrow a DocModel beside a UI-thread mutation.
            editor.post(this::runHistorySmoke);
        }
    }

    private void runHistorySmoke() {
        if (handle == 0 || isDestroyed()) return;
        try {
            String before = DocBridge.state(handle);
            String remembered = DocBridge.historyRemember(handle, "device");
            int count = new JSONObject(remembered).getInt("count");
            JSONArray tokens = new JSONObject(DocBridge.historyFrontiers(handle))
                    .getJSONArray("frontiers");
            JSONObject preview = new JSONObject(
                    DocBridge.historyPreview(handle, tokens.getString(tokens.length() - 1)));
            if (count != tokens.length() || preview.getString("digest").isEmpty()
                    || !before.equals(DocBridge.state(handle))) {
                throw new IllegalStateException("history changed the live document");
            }
            Log.d("emthin-pad", "history smoke: count=" + count + " readOnly=true");
        } catch (Exception e) {
            Log.e("emthin-pad", "history smoke failed", e);
        }
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
        // A launch fixture names the document to open, so it outranks whatever
        // this device had open before: the lane restarts the same build again and
        // again, and the recovery copy the previous launch left behind is not the
        // document it asked for. `fixtureText` is only ever set by the test APK.
        if (fixtureText != null) {
            installFreshStarter();
            return;
        }
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
                    setStatus("Recovered an earlier checkpoint · recovery evidence retained");
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
        if (inspectorOpen) updateModifierPanel();
        scheduleRender();
        requestViewWrite(true);
        if (legacyPending) captureRecoveryNow();
        resetNavigation();
    }

    private void installFreshStarter() {
        if (handle != 0) DocBridge.free(handle);
        boolean fixture = fixturePending;
        fixturePending = false;
        handle = DocBridge.create(fixture && fixtureText != null ? fixtureText : STARTER);

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
        setStatus("New document");
        captureRecoveryNow();
        resetNavigation();
        if (fixture) installModifierFixture();
        if (sessionPinFixturePending) runSessionPinFixture();
        if (fixtureOpenPath != null) {
            String path = fixtureOpenPath;
            fixtureOpenPath = null;
            Uri uri = Uri.fromFile(new java.io.File(path));
            setDocumentLabel(uri.getLastPathSegment());
            readDocument(uri, session.capture(PadSession.OPEN, "fixtureOpen"));
        }
    }

    /**
     * Device-lane fixture: pin the first live session, then remove and restore
     * the figure with Undo/Redo before the normal autosave/reopen check. The
     * public reference is the only catalogue field used.
     */
    private void runSessionPinFixture() {
        sessionPinFixturePending = false;
        // The pin step runs without a gateway, so it is handed the scripted
        // catalogue; the live one comes from `sessions.list` and would be
        // empty here.
        sessionCatalog = SessionCatalog.fixture();
        if (sessionCatalog.entries().isEmpty()) {
            trace("sessionPin", "phase", "failed", "reason", "empty-catalogue");
            return;
        }
        SessionCatalog.Entry entry = sessionCatalog.entries().get(0);
        String reference = entry.reference;
        pinSession(entry);
        if (!editor.getText().toString().contains(reference)) {
            trace("sessionPin", "phase", "failed", "reason", "pin-not-applied");
            return;
        }
        trace("sessionPin", "phase", "pinned", "textLen", editor.length(),
                "reference", reference);

        applyUndo();
        if (editor.getText().toString().contains(reference)) {
            trace("sessionPin", "phase", "failed", "reason", "undo-kept-reference");
            return;
        }
        trace("sessionPin", "phase", "undo", "textLen", editor.length());

        applyState(DocBridge.redo(handle), true);
        if (!editor.getText().toString().contains(reference)) {
            trace("sessionPin", "phase", "failed", "reason", "redo-lost-reference");
            return;
        }
        trace("sessionPin", "phase", "complete", "textLen", editor.length(),
                "reference", reference);
    }

    /**
     * Device-lane fixture: query the rendered figure node, prove its state and
     * custom actions, then activate focus, go, detach and Sessions through the
     * accessibility provider rather than by tapping the page.
     */
    private void runFigureA11yFixture(RenderedPage page) {
        figureA11yFixturePending = false;
        // Like the pin step, this fixture runs with no gateway; the figure has
        // to resolve to a catalogue row or it would only ever be Unavailable.
        sessionCatalog = SessionCatalog.fixture();
        int index = -1;
        for (int i = 0; i < page.figures.length(); i++) {
            JSONObject figure = page.figures.optJSONObject(i);
            if (figure != null && !jsonString(figure, "reference").isEmpty()) {
                index = i;
                break;
            }
        }
        if (index < 0) {
            trace("figureA11y", "phase", "failed", "reason", "no-terminal-figure");
            return;
        }
        int id = RenderedPage.FIGURE_NODE_BASE + index;
        android.view.accessibility.AccessibilityNodeProvider provider =
                page.getAccessibilityNodeProvider();
        android.view.accessibility.AccessibilityNodeInfo info =
                provider.createAccessibilityNodeInfo(id);
        if (info == null) {
            trace("figureA11y", "phase", "failed", "reason", "no-node");
            return;
        }
        String description = String.valueOf(info.getContentDescription());
        trace("figureA11y", "phase", "nodes", "description", description,
                "go", hasFigureAction(info, RenderedPage.ACTION_FIGURE_GO),
                "rebind", hasFigureAction(info, RenderedPage.ACTION_FIGURE_REBIND),
                "detach", hasFigureAction(info, RenderedPage.ACTION_FIGURE_DETACH),
                "sessions", hasFigureAction(info, RenderedPage.ACTION_FIGURE_SESSIONS));
        if (!description.contains("Watching")
                || !hasFigureAction(info, RenderedPage.ACTION_FIGURE_GO)
                || !hasFigureAction(info, RenderedPage.ACTION_FIGURE_REBIND)
                || !hasFigureAction(info, RenderedPage.ACTION_FIGURE_DETACH)
                || !hasFigureAction(info, RenderedPage.ACTION_FIGURE_SESSIONS)) {
            trace("figureA11y", "phase", "failed", "reason", "node-contract");
            return;
        }
        trace("figureA11y", "phase", "focus-request");
        if (!provider.performAction(id,
                android.view.accessibility.AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS, null)) {
            trace("figureA11y", "phase", "failed", "reason", "focus-action");
            return;
        }
        trace("figureA11y", "phase", "focus-action");
        android.view.accessibility.AccessibilityNodeInfo focused = provider.findFocus(
                android.view.accessibility.AccessibilityNodeInfo.FOCUS_ACCESSIBILITY);
        trace("figureA11y", "phase", "focus-found", "node", focused != null);
        if (focused == null || !String.valueOf(focused.getContentDescription()).contains("Watching")) {
            trace("figureA11y", "phase", "failed", "reason", "focus-not-found");
            return;
        }
        trace("figureA11y", "phase", "focus", "description", focused.getContentDescription());
        if (!provider.performAction(id, RenderedPage.ACTION_FIGURE_GO, null)) {
            trace("figureA11y", "phase", "failed", "reason", "go-action");
            return;
        }
        trace("figureA11y", "phase", "go");
        if (!provider.performAction(id, RenderedPage.ACTION_FIGURE_DETACH, null)) {
            trace("figureA11y", "phase", "failed", "reason", "detach-action");
            return;
        }
        info = provider.createAccessibilityNodeInfo(id);
        String detached = info == null ? "" : String.valueOf(info.getContentDescription());
        if (!detached.contains("Live on")) {
            trace("figureA11y", "phase", "failed", "reason", "detach-state",
                    "description", detached);
            return;
        }
        trace("figureA11y", "phase", "detach", "description", detached);
        if (!provider.performAction(id, RenderedPage.ACTION_FIGURE_SESSIONS, null)) {
            trace("figureA11y", "phase", "failed", "reason", "sessions-action");
            return;
        }
        trace("figureA11y", "phase", "sessions");
        trace("figureA11y", "phase", "complete");
    }

    /** Exercise the terminal figure's accessibility actions through the
     *  provider, the same path an accessibility service uses. */
    private void runVtSceneA11yFixture(RenderedPage page, int index) {
        if (!vtSceneA11yFixture) return;
        vtSceneA11yFixture = false;
        int id = RenderedPage.FIGURE_NODE_BASE + index;
        android.view.accessibility.AccessibilityNodeProvider provider =
                page.getAccessibilityNodeProvider();
        android.view.accessibility.AccessibilityNodeInfo info =
                provider.createAccessibilityNodeInfo(id);
        boolean focus = info != null && hasFigureAction(info,
                RenderedPage.ACTION_FIGURE_TERMINAL_FOCUS);
        boolean copy = info != null && hasFigureAction(info,
                RenderedPage.ACTION_FIGURE_TERMINAL_COPY);
        boolean paste = info != null && hasFigureAction(info,
                RenderedPage.ACTION_FIGURE_TERMINAL_PASTE);
        trace("vtA11y", "phase", "node", "focus", focus, "copy", copy, "paste", paste);
        if (!focus || !copy || !paste) {
            trace("vtA11y", "phase", "failed", "reason", "actions");
            return;
        }
        if (!provider.performAction(id, RenderedPage.ACTION_FIGURE_TERMINAL_FOCUS, null)) {
            trace("vtA11y", "phase", "failed", "reason", "focus-action");
            return;
        }
        if (!provider.performAction(id, RenderedPage.ACTION_FIGURE_TERMINAL_COPY, null)) {
            trace("vtA11y", "phase", "failed", "reason", "copy-action");
            return;
        }
        android.content.ClipboardManager clipboard =
                (android.content.ClipboardManager) getSystemService(CLIPBOARD_SERVICE);
        int chars = 0;
        if (clipboard != null && clipboard.hasPrimaryClip()
                && clipboard.getPrimaryClip() != null
                && clipboard.getPrimaryClip().getItemCount() > 0) {
            CharSequence value = clipboard.getPrimaryClip().getItemAt(0).coerceToText(this);
            chars = value == null ? 0 : value.length();
        }
        trace("vtA11y", "phase", "copy", "chars", chars);
        if (chars == 0) {
            trace("vtA11y", "phase", "failed", "reason", "empty-copy");
            return;
        }
        // The initial fixture session is watched. Take control before the
        // paste action so it reaches the scene rather than the read-only gate.
        focusTerminal(page, index);
        setTerminalControl(false);
        if (clipboard != null) clipboard.setPrimaryClip(
                android.content.ClipData.newPlainText("Terminal", "a11y-paste"));
        if (!provider.performAction(id, RenderedPage.ACTION_FIGURE_TERMINAL_PASTE, null)) {
            trace("vtA11y", "phase", "failed", "reason", "paste-action");
            return;
        }
        // A conflicting lease is also read-only, but it must be shown as a
        // conflict rather than silently looking like ordinary watching.
        page.terminalConflict[index] = true;
        page.invalidate();
        trace("terminalInput", "control", "conflict",
                "page", page.index, "index", index);
        if (!provider.performAction(id, RenderedPage.ACTION_FIGURE_TERMINAL_PASTE, null)) {
            trace("vtA11y", "phase", "failed", "reason", "conflict-paste-action");
            return;
        }
        trace("vtA11y", "phase", "complete");
    }

    /** Android's optString turns a JSON null into the text "null". */
    private static String jsonString(JSONObject object, String key) {
        return object == null || object.isNull(key) ? "" : object.optString(key, "");
    }

    private static boolean hasFigureAction(
            android.view.accessibility.AccessibilityNodeInfo info, int id) {
        for (android.view.accessibility.AccessibilityNodeInfo.AccessibilityAction action
                : info.getActionList()) {
            if (action.getId() == id) return true;
        }
        return false;
    }

    /** Test-only hook: link a modifier pair to target pairs and open the inspector. */
    private void installModifierFixture() {
        if (fixtureModifier == null || fixtureTargets == null) return;
        String[] ids = fixtureModifier.trim().split("\\s*,\\s*|\\s+");
        if (ids.length != 2) return;
        String result = DocBridge.setMarkerTargets(handle, ids[0], ids[1], fixtureTargets);
        try {
            JSONObject state = new JSONObject(result);
            if (state.has("error")) {
                trace("fixture", "error", state.optString("error"));
                return;
            }
            if (fixtureModifier2 != null && fixtureTargets2 != null) {
                String[] incoming = fixtureModifier2.trim().split("\\s*,\\s*|\\s+");
                if (incoming.length == 2) {
                    result = DocBridge.setMarkerTargets(handle, incoming[0], incoming[1], fixtureTargets2);
                    state = new JSONObject(result);
                    if (state.has("error")) {
                        trace("fixture", "error", state.optString("error"));
                        return;
                    }
                }
            }
            applyState(result, false);
            int caret = 0;
            String wanted = fixtureMarker == null ? ids[0] : fixtureMarker;
            for (int i = 0; i < documentMarkers.length(); i++) {
                JSONObject marker = documentMarkers.optJSONObject(i);
                if (marker != null && wanted.equals(marker.optString("id"))) {
                    caret = marker.optInt("end", 0);
                    break;
                }
            }
            inspectorOpen = true;
            wholeDocumentEditing = false;
            editor.setSelection(caret);
            inspectorSignature = "";
            // Same layout as tapping Edit, so the lane measures the pane a user
            // actually gets.
            previewScroll.setVisibility(View.GONE);
            updateModifierPanel();
            focusSourceCaret();
            trace("fixture", "modifier", fixtureModifier, "targets", fixtureTargets,
                    "caret", caret);
        } catch (Exception e) {
            trace("fixture", "error", e.toString());
        }
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
                        setStatus("Recovered text without marks · original snapshot retained");
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
        setStatus("Recovery needs a choice");
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
        // Every committed change passes through here, so this is the one place
        // a local edit can reach an open share. Re-entrant calls come from the
        // remote-apply path and must not publish what we just imported.
        if (!publishingShare) publishShare();
    }

    /** Publish the live document's delta to the sharing worker, then drain. */
    private void publishShare() {
        if (shareHandle == 0 || shareWatchOnly || handle == 0) return;
        publishingShare = true;
        try {
            org.json.JSONObject reply = new org.json.JSONObject(
                    DocBridge.shareSync(handle, shareHandle));
            if (!reply.optBoolean("ok", false)) {
                trace("share", "publish_failed", reply.optString("error", ""));
            }
            pollShare();
        } catch (org.json.JSONException e) {
            trace("share", "publish_failed", e.getMessage());
        } finally {
            publishingShare = false;
        }
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
            setStatus("Recovery cannot be updated: document exceeds 32 MiB");
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
                setStatus("Recovery failed: " + failure);
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
            setStatus(unsaved ? "Recovery copy saved on this device" : "");
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
        closeGatewayLease();
        if (shareHandle != 0) {
            DocBridge.shareClose(shareHandle);
            shareHandle = 0;
        }
        session.dispose();
        if (find != null) find.cancel();
        renderWork.close();
        findWorker.shutdown();
        main.removeCallbacks(renderNow);
        main.removeCallbacks(updateModifierComparisons);
        main.removeCallbacks(recoveryNow);
        main.removeCallbacks(viewNow);
        fileWorker.shutdown();
        // Already-captured bytes may finish; they never touch this view.
        recoveryWorker.shutdown();
        releasePageBitmaps();
        for (TerminalTransfer transfer : pendingTerminalTransfers) {
            transfer.scene.close();
        }
        pendingTerminalTransfers.clear();
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

        buildToolbar(root);
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
        terminalSelectionBar = new LinearLayout(this);
        terminalSelectionBar.setOrientation(LinearLayout.HORIZONTAL);
        terminalSelectionBar.setBackgroundColor(Color.parseColor("#1A2229"));
        terminalSelectionBar.setVisibility(View.GONE);
        terminalSelectionBar.addView(shortcut("Copy", new Runnable() {
            @Override public void run() { copyTerminalSelection(); }
        }));
        terminalSelectionBar.addView(shortcut("Paste", new Runnable() {
            @Override public void run() { pasteTerminalClipboard(); }
        }));
        terminalSelectionBar.addView(shortcut("Close", new Runnable() {
            @Override public void run() { clearTerminalSelection(); }
        }));
        root.addView(terminalSelectionBar, new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT));
        searchBar = new LinearLayout(this);
        searchBar.setOrientation(LinearLayout.VERTICAL);
        searchBar.setVisibility(View.GONE);
        LinearLayout searchQueryRow = new LinearLayout(this);
        searchQueryRow.setOrientation(LinearLayout.HORIZONTAL);
        searchField = new EditText(this);
        searchField.setSingleLine(true);
        searchField.setTextColor(Color.parseColor("#E6EDF3"));
        searchField.setHintTextColor(Color.parseColor("#8796A5"));
        searchField.setHint("Find · Enter accept · ^S next here · ^R previous · Esc cancel");
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
                if (actionId == EditorInfo.IME_ACTION_DONE) { acceptFind(); return true; }
                return false;
            }
        });
        searchQueryRow.addView(searchField, new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1));
        findScopeButton = shortcut("Text", new Runnable() {
            @Override public void run() { cycleFindScope(); }
        });
        findScopeButton.setContentDescription("Search scope: Text");
        searchQueryRow.addView(findScopeButton);
        searchCount = panelLabel("");
        searchQueryRow.addView(searchCount);
        searchBar.addView(searchQueryRow);

        LinearLayout searchNavRow = new LinearLayout(this);
        searchNavRow.setOrientation(LinearLayout.HORIZONTAL);
        searchNavRow.addView(shortcut("^R Prev", new Runnable() {
            @Override public void run() { moveFind(-1); }
        }));
        searchNavRow.addView(shortcut(ActionCatalog.label(ActionCatalog.Id.FIND_NEXT,
                false), new Runnable() {
            @Override public void run() { runAction(ActionCatalog.Id.FIND_NEXT); }
        }));
        searchNavRow.addView(shortcut("Enter Accept", new Runnable() {
            @Override public void run() { acceptFind(); }
        }));
        searchNavRow.addView(shortcut("Esc Cancel", new Runnable() {
            @Override public void run() { closeSearch(true); }
        }));
        searchNavRow.addView(shortcut("Replace…", new Runnable() {
            @Override public void run() { toggleReplaceRow(); }
        }));
        searchBar.addView(searchNavRow);

        replaceRow = new LinearLayout(this);
        replaceRow.setOrientation(LinearLayout.HORIZONTAL);
        replaceRow.setVisibility(View.GONE);
        replaceField = new EditText(this);
        replaceField.setSingleLine(true);
        replaceField.setTextColor(Color.parseColor("#E6EDF3"));
        replaceField.setHintTextColor(Color.parseColor("#8796A5"));
        replaceField.setHint("Replace with literal text");
        replaceField.setContentDescription("Replacement text");
        replaceField.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS);
        replaceField.addTextChangedListener(new TextWatcher() {
            @Override public void beforeTextChanged(CharSequence s, int start, int count, int after) {}
            @Override public void onTextChanged(CharSequence s, int start, int before, int count) {}
            @Override public void afterTextChanged(Editable s) {
                if (replacePlan != null) {
                    clearReplacePlan();
                    replaceStatus.setText("");
                }
            }
        });
        replaceRow.addView(replaceField, new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1));
        replaceRow.addView(shortcut("Replace", new Runnable() {
            @Override public void run() { replaceCurrent(); }
        }));
        replaceAllButton = shortcut("Replace all…", new Runnable() {
            @Override public void run() { previewOrApplyReplaceAll(); }
        });
        replaceRow.addView(replaceAllButton);
        replaceStatus = panelLabel("");
        replaceStatus.setVisibility(View.GONE);
        searchBar.addView(replaceRow);
        searchBar.addView(replaceStatus);

        findResultsScroll = new ScrollView(this);
        findResultsScroll.setVisibility(View.GONE);
        findResults = new LinearLayout(this);
        findResults.setOrientation(LinearLayout.VERTICAL);
        findResultsScroll.addView(findResults);
        searchBar.addView(findResultsScroll, new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, dp(150)));
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
        // Each page is measured at its document aspect ratio. The viewport
        // scrolls over that stack at both fit width and higher zoom levels.
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
                for (RenderedPage page : renderedPages) page.clipTerminalSurfaces();
                rememberReadingAnchor();
            }
        });
        // The IME changes the preview viewport without a scroll. Recompute
        // the z-ordered scene clip or it keeps the keyboard-shrunk height
        // after the keyboard closes.
        previewScroll.addOnLayoutChangeListener(new View.OnLayoutChangeListener() {
            @Override public void onLayoutChange(View v, int left, int top, int right, int bottom,
                    int oldLeft, int oldTop, int oldRight, int oldBottom) {
                if (right - left == oldRight - oldLeft
                        && bottom - top == oldBottom - oldTop) return;
                for (RenderedPage page : renderedPages) page.clipTerminalSurfaces();
            }
        });
        // The window's available height can change around the IME without
        // this view's own layout rectangle changing. Observe the settled
        // viewport size and re-clip the z-ordered surfaces when it differs.
        final int[] lastViewport = new int[] {-1, -1};
        previewScroll.getViewTreeObserver().addOnGlobalLayoutListener(
                new ViewTreeObserver.OnGlobalLayoutListener() {
            @Override public void onGlobalLayout() {
                int width = previewScroll.getWidth();
                int height = previewScroll.getHeight();
                if (width == lastViewport[0] && height == lastViewport[1]) return;
                lastViewport[0] = width;
                lastViewport[1] = height;
                for (RenderedPage page : renderedPages) page.clipTerminalSurfaces();
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

        // A terminal-focused key row supplies named keys the soft keyboard does
        // not expose. It stays out of the document layout until a figure owns
        // focus, so the normal reader keeps its full height.
        terminalKeyRow = new LinearLayout(this);
        terminalKeyRow.setOrientation(LinearLayout.HORIZONTAL);
        terminalKeyRow.setVisibility(View.GONE);
        terminalKeyRow.setBackgroundColor(Color.parseColor("#202830"));
        terminalKeyRow.addView(terminalKeyButton("Esc", KeyEvent.KEYCODE_ESCAPE));
        terminalKeyRow.addView(terminalKeyButton("Tab", KeyEvent.KEYCODE_TAB));
        terminalKeyRow.addView(terminalKeyButton("←", KeyEvent.KEYCODE_DPAD_LEFT));
        terminalKeyRow.addView(terminalKeyButton("↑", KeyEvent.KEYCODE_DPAD_UP));
        terminalKeyRow.addView(terminalKeyButton("↓", KeyEvent.KEYCODE_DPAD_DOWN));
        terminalKeyRow.addView(terminalKeyButton("→", KeyEvent.KEYCODE_DPAD_RIGHT));
        terminalKeyRow.addView(terminalKeyButton("Ctrl-C", KeyEvent.KEYCODE_C, true));
        terminalKeyRow.addView(shortcut("Watch", new Runnable() {
            @Override public void run() { setTerminalControl(true); }
        }));
        terminalKeyRow.addView(shortcut("Take", new Runnable() {
            @Override public void run() { setTerminalControl(false); }
        }));
        terminalKeyRow.addView(shortcut("Done", new Runnable() {
            @Override public void run() { releaseTerminalFocus(); }
        }));
        root.addView(terminalKeyRow, new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.WRAP_CONTENT));

        // The inspector needs room for its relationship list. An equal split
        // with the preview leaves only ~118dp on a short landscape tablet,
        // which clips every row after the first.
        root.addView(modifierPanel, new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, 0, 2f));

        setContentView(root);
    }

    private void buildToolbar(LinearLayout root) {
        Configuration configuration = getResources().getConfiguration();
        boolean twoRows = configuration.screenWidthDp < 360 || configuration.fontScale >= 1.35f;
        LinearLayout row = addToolbarRow(root, configuration.screenWidthDp);
        for (int i = 0; i < ActionCatalog.TOOLBAR.length; i++) {
            if (twoRows && i == 3) row = addToolbarRow(root, configuration.screenWidthDp);
            ActionCatalog.Id action = ActionCatalog.TOOLBAR[i];
            TextView button = shortcut(ActionCatalog.label(action, false), () -> runAction(action));
            if (action == ActionCatalog.Id.UNDO) {
                undoButton = button;
                undoButton.setEnabled(false);
            }
            addToolbarButton(row, button);
        }
        TextView more = new TextView(this);
        more.setText("More");
        more.setContentDescription("More actions");
        more.setOnClickListener(this::showToolbarMenu);
        addToolbarButton(row, more);
    }

    private LinearLayout addToolbarRow(LinearLayout root, int widthDp) {
        LinearLayout row = new LinearLayout(this);
        row.setBaselineAligned(false);
        int width = widthDp >= 456 ? dp(432) : LinearLayout.LayoutParams.MATCH_PARENT;
        root.addView(row, new LinearLayout.LayoutParams(width, LinearLayout.LayoutParams.WRAP_CONTENT));
        return row;
    }

    private void addToolbarButton(LinearLayout row, TextView button) {
        button.setGravity(Gravity.CENTER);
        button.setTextSize(13f);
        button.setMinHeight(dp(48));
        button.setPadding(dp(3), dp(4), dp(3), dp(4));
        button.setFocusable(true);
        button.setTextColor(new android.content.res.ColorStateList(
                new int[][] {new int[] {-android.R.attr.state_enabled}, new int[0]},
                new int[] {0xff71808e, 0xffdce7ef}));
        row.addView(button, new LinearLayout.LayoutParams(0,
                LinearLayout.LayoutParams.WRAP_CONTENT, 1f));
    }

    private void showToolbarMenu(View anchor) {
        PopupMenu menu = new PopupMenu(this, anchor);
        addToolbarMenu(menu.getMenu().addSubMenu("Document"), ActionCatalog.DOCUMENT_MENU);
        addToolbarMenu(menu.getMenu().addSubMenu("Navigation"), ActionCatalog.NAVIGATION_MENU);
        addToolbarMenu(menu.getMenu().addSubMenu("View"), ActionCatalog.VIEW_MENU);
        addToolbarMenu(menu.getMenu().addSubMenu("Sessions and sharing"), ActionCatalog.SESSIONS_MENU);
        menu.show();
    }

    private void addToolbarMenu(SubMenu menu, ActionCatalog.Id[] actions) {
        for (ActionCatalog.Id action : actions) {
            boolean enabled = true;
            switch (action) {
                case REDO: enabled = canRedo; break;
                case NAV_BACK: enabled = navigation.canBack(); break;
                case NAV_FORWARD: enabled = navigation.canForward(); break;
                case CLOSE_INSPECTOR: enabled = inspectorOpen; break;
                case DIAGNOSTICS: enabled = currentDiagnostics.length() > 0; break;
                default: break;
            }
            menu.add(ActionCatalog.label(action, showMarkers)).setEnabled(enabled)
                    .setOnMenuItemClickListener(item -> runAction(action));
        }
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
        if (requestCode == REQUEST_QR) {
            completeQrScan(resultCode, data);
            return;
        }
        if (requestCode == REQUEST_PROOF) {
            if (resultCode != RESULT_OK || data == null) return;
            String proofId = data.getStringExtra("emthin.proof_id");
            String proofLink = data.getStringExtra("emthin.proof_link");
            if (proofId == null || proofId.isEmpty()) return;
            insertProofResult(proofId, proofLink == null ? "" : proofLink);
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

    /**
     * Insert a proof result into the note as ordinary prose at the caret. The
     * editor watcher funnels the change through DocBridge.sync, so this is one
     * grouped edit and follows the same undo/recovery path as typing.
     */
    private void insertProofResult(String proofId, String proofLink) {
        String line = "\nProof ID: " + proofId
                + (proofLink.isEmpty() ? "" : "\nResult link: " + proofLink) + "\n";
        Editable text = editor.getText();
        int at = editor.getSelectionStart();
        if (at < 0 || at > text.length()) at = text.length();
        text.insert(at, line);
        editor.setSelection(at + line.length());
        setStatus("Proof " + proofId + " inserted");
        trace("proof", "inserted", proofId);
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
        documentLabel = uri.getLastPathSegment();
        documentIsSnapshot = snapshot;
        fileStamp = stamp;
        SharedPreferences.Editor edit = prefs().edit()
                .putString(PREF_DOCUMENT_URI, uri.toString())
                .putString("documentLabel", documentLabel)
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
        // Keep documentLabel: a source-only Open has no save URI until Save,
        // but the status line should still name the file that was opened.
    }

    private void clearDocumentLabel() {
        documentLabel = null;
        prefs().edit().remove("documentLabel").apply();
    }

    private void setDocumentLabel(String label) {
        documentLabel = label;
        prefs().edit().putString("documentLabel", label).apply();
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
                setStatus("Save queued · the newest capture will be written");
            } else if (waitingFile == null) {
                waitingFile = op;
                setStatus("Waiting for the current file operation…");
            } else {
                fileMessage("Another file operation is already waiting; try again when it finishes.");
            }
            return;
        }
        startFileOperation(op);
    }

    private void startFileOperation(final FileOperation op) {
        activeFile = op;
        setStatus("save".equals(op.kind)
                ? (op.copy ? "Writing copy…" : "Writing snapshot…")
                : "Opening file…");
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
                            if (session.sameDocument(op.owner)) {
                                setStatus("Open failed · recovery retained");
                                fileMessage("Could not open document: " + error);
                            }
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
            setStatus("Open cancelled · document changed");
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
                setStatus("Open failed · malformed snapshot");
                fileMessage("Could not open this document: malformed .mathed snapshot.");
                return;
            }
            try {
                String text = ProviderIo.decodeUtf8(bytes);
                next = DocBridge.create(text);
            } catch (java.nio.charset.CharacterCodingException e) {
                setStatus("Open failed · invalid UTF-8");
                fileMessage("Could not open this document: invalid UTF-8.");
                return;
            }
        }
        if (next == 0) {
            setStatus("Open failed · recovery retained");
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
                    setStatus("Open cancelled · document changed");
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
                    // require Save to create a .mathed snapshot, but preserve
                    // the opened file's name in the status line.
                    setDocumentLabel(op.uri.getLastPathSegment());
                    forgetDocument();
                    savedFingerprint = "";
                    unsaved = true;
                }
                durableFingerprint = "";
                restoringState = true;
                applyState(DocBridge.state(handle), true);
                restoringState = false;
                resetNavigation();
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
        setStatus("Starting queued file operation…");
        startFileOperation(next);
    }

    private void saveSucceeded(FileOperation op, ProviderIo.Stamp stamp, String fingerprint) {
        if (!session.sameDocument(op.owner)) return;
        if (op.copy) {
            setStatus("Copy saved · active document unchanged");
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
            setStatus("");
            fileMessage("Document saved");
        } else {
            fileMessage("An earlier revision was saved; newer edits remain unsaved.");
        }
    }

    private void saveFailed(FileOperation op, String error) {
        if (!session.sameDocument(op.owner)) return;
        if (op.copy) {
            setStatus("Copy failed · active document unchanged");
            fileMessage("Could not save copy: " + error + ". The active file and recovery are unchanged.");
            return;
        }
        unsaved = true;
        setStatus("Save failed · recovery retained");
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
            setStatus("save".equals(op.kind)
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
        // New retains its existing undo behavior, but read-only history
        // belongs to the previous document instance.
        DocBridge.historyClear(handle);
        forgetDocument();
        clearDocumentLabel();
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
        resetNavigation();
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

    /**
     * A terminal scene submitted to a real GLES window surface. The native
     * scene still composes its RGBA raster; this view owns the EGL14 window
     * surface and uploads that raster into the page's texture layer instead
     * of blitting it through Canvas.
     */
    private final class TerminalSurface extends SurfaceView implements SurfaceHolder.Callback {
        private final int pageIndex;
        private final NativeTerminal scene;
        private int texture;
        private EGLDisplay eglDisplay = EGL14.EGL_NO_DISPLAY;
        private EGLContext eglContext = EGL14.EGL_NO_CONTEXT;
        private EGLSurface eglSurface = EGL14.EGL_NO_SURFACE;
        private EGLConfig eglConfig;
        private Surface eglWindow;
        private int program;
        private int positionSlot = -1;
        private int uvSlot = -1;
        private int textureSlot = -1;
        private int bufferWidth;
        private int bufferHeight;
        private float u0, v0, u1 = 1f, v1 = 1f;
        private boolean glReady;
        private boolean rendered;
        private boolean repaintRequested;
        private String repaintReason = "layout";
        private boolean framePosted;
        private boolean failed;

        TerminalSurface(android.content.Context context, int pageIndex,
                NativeTerminal scene) {
            super(context);
            this.pageIndex = pageIndex;
            this.scene = scene;
            setZOrderOnTop(true);
            setClickable(false);
            setFocusable(false);
            setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO);
            getHolder().addCallback(this);
        }

        boolean submitting() {
            return rendered;
        }

        /** Force one fresh submission after the surface was hidden or detached. */
        void repaint(String reason) {
            rendered = false;
            repaintRequested = true;
            repaintReason = reason;
            requestFrame();
        }

        void requestFrame() {
            if (failed || framePosted || !surfaceReady()) return;
            framePosted = true;
            postOnAnimation(new Runnable() {
                @Override public void run() {
                    framePosted = false;
                    renderFrame();
                }
            });
        }

        /** Drop the EGL objects so the fixture can prove a context loss. */
        /** Visible fraction of the full scene texture for a clipped surface. */
        void setCrop(float left, float top, float right, float bottom) {
            u0 = Math.max(0f, Math.min(1f, left));
            v0 = Math.max(0f, Math.min(1f, top));
            u1 = Math.max(u0, Math.min(1f, right));
            v1 = Math.max(v0, Math.min(1f, bottom));
        }

        void recreateContext() {
            if (!surfaceReady()) return;
            releaseGl();
            if (setupGl(eglWindow)) requestFrame();
        }

        void release() {
            releaseGl();
            eglWindow = null;
        }

        @Override public void surfaceCreated(SurfaceHolder holder) {
            eglWindow = holder.getSurface();
            if (setupGl(eglWindow)) requestFrame();
        }

        @Override public void surfaceChanged(SurfaceHolder holder, int format,
                int width, int height) {
            eglWindow = holder.getSurface();
            requestFrame();
        }

        @Override public void surfaceDestroyed(SurfaceHolder holder) {
            releaseGl();
            eglWindow = null;
        }

        @Override protected void onDetachedFromWindow() {
            releaseGl();
            super.onDetachedFromWindow();
        }

        private boolean surfaceReady() {
            return eglWindow != null && eglWindow.isValid();
        }

        private boolean setupGl(Surface target) {
            if (failed) return false;
            releaseGl();
            try {
                eglDisplay = EGL14.eglGetDisplay(EGL14.EGL_DEFAULT_DISPLAY);
                if (eglDisplay == EGL14.EGL_NO_DISPLAY) throw new IllegalStateException("no EGL display");
                int[] version = new int[2];
                if (!EGL14.eglInitialize(eglDisplay, version, 0, version, 1)) {
                    throw new IllegalStateException("eglInitialize failed");
                }
                int[] attributes = new int[] {
                        EGL14.EGL_RENDERABLE_TYPE, EGL14.EGL_OPENGL_ES2_BIT,
                        EGL14.EGL_SURFACE_TYPE, EGL14.EGL_WINDOW_BIT,
                        EGL14.EGL_RED_SIZE, 8,
                        EGL14.EGL_GREEN_SIZE, 8,
                        EGL14.EGL_BLUE_SIZE, 8,
                        EGL14.EGL_ALPHA_SIZE, 8,
                        EGL14.EGL_NONE,
                };
                EGLConfig[] configs = new EGLConfig[1];
                int[] selected = new int[1];
                if (!EGL14.eglChooseConfig(eglDisplay, attributes, 0, configs, 0, 1, selected, 0)
                        || selected[0] == 0) {
                    throw new IllegalStateException("no EGL window config");
                }
                eglConfig = configs[0];
                eglContext = EGL14.eglCreateContext(eglDisplay, eglConfig,
                        EGL14.EGL_NO_CONTEXT,
                        new int[] {EGL14.EGL_CONTEXT_CLIENT_VERSION, 2, EGL14.EGL_NONE}, 0);
                if (eglContext == EGL14.EGL_NO_CONTEXT) {
                    throw new IllegalStateException("eglCreateContext failed");
                }
                eglSurface = EGL14.eglCreateWindowSurface(eglDisplay, eglConfig,
                        target, new int[] {EGL14.EGL_NONE}, 0);
                if (eglSurface == EGL14.EGL_NO_SURFACE) {
                    throw new IllegalStateException("eglCreateWindowSurface failed");
                }
                if (!EGL14.eglMakeCurrent(eglDisplay, eglSurface, eglSurface, eglContext)) {
                    throw new IllegalStateException("eglMakeCurrent failed");
                }
                int[] textures = new int[1];
                GLES20.glGenTextures(1, textures, 0);
                texture = textures[0];
                program = GLES20.glCreateProgram();
                int vertex = compileShader(GLES20.GL_VERTEX_SHADER,
                        "attribute vec2 a_pos;\n"
                                + "attribute vec2 a_uv;\n"
                                + "varying vec2 v_uv;\n"
                                + "void main(){\n"
                                + "  gl_Position=vec4(a_pos,0.0,1.0);\n"
                                + "  v_uv=a_uv;\n"
                                + "}\n");
                int fragment = compileShader(GLES20.GL_FRAGMENT_SHADER,
                        "precision mediump float;\n"
                                + "uniform sampler2D u_texture;\n"
                                + "varying vec2 v_uv;\n"
                                + "void main(){gl_FragColor=texture2D(u_texture,v_uv);}\n");
                GLES20.glAttachShader(program, vertex);
                GLES20.glAttachShader(program, fragment);
                GLES20.glLinkProgram(program);
                int[] linked = new int[1];
                GLES20.glGetProgramiv(program, GLES20.GL_LINK_STATUS, linked, 0);
                GLES20.glDeleteShader(vertex);
                GLES20.glDeleteShader(fragment);
                if (linked[0] == 0) {
                    throw new IllegalStateException("shader link: "
                            + GLES20.glGetProgramInfoLog(program));
                }
                positionSlot = GLES20.glGetAttribLocation(program, "a_pos");
                uvSlot = GLES20.glGetAttribLocation(program, "a_uv");
                textureSlot = GLES20.glGetUniformLocation(program, "u_texture");
                glReady = true;
                rendered = false;
                failed = false;
                trace("vtScene", "gl", "surface", "page", pageIndex,
                        "texture", textureId());
                return true;
            } catch (RuntimeException error) {
                trace("vtScene", "gl", "failed", "page", pageIndex,
                        "reason", error.getMessage());
                releaseGl();
                return false;
            }
        }

        private int textureId() {
            return texture;
        }

        private int compileShader(int type, String source) {
            int shader = GLES20.glCreateShader(type);
            GLES20.glShaderSource(shader, source);
            GLES20.glCompileShader(shader);
            int[] compiled = new int[1];
            GLES20.glGetShaderiv(shader, GLES20.GL_COMPILE_STATUS, compiled, 0);
            if (compiled[0] == 0) {
                String log = GLES20.glGetShaderInfoLog(shader);
                GLES20.glDeleteShader(shader);
                throw new IllegalStateException("shader compile: " + log);
            }
            return shader;
        }

        private void renderFrame() {
            if (!glReady || failed || !surfaceReady() || scene == null) return;
            Bitmap bitmap = scene.bitmap();
            if (bitmap == null || bitmap.isRecycled()) return;
            int width = bitmap.getWidth();
            int height = bitmap.getHeight();
            if (width <= 0 || height <= 0) return;
            if (!EGL14.eglMakeCurrent(eglDisplay, eglSurface, eglSurface, eglContext)) {
                releaseGl();
                return;
            }
            try {
                bufferWidth = width;
                bufferHeight = height;
                GLES20.glViewport(0, 0, Math.max(1, getWidth()), Math.max(1, getHeight()));
                GLES20.glDisable(GLES20.GL_DEPTH_TEST);
                GLES20.glClearColor(0.078f, 0.094f, 0.114f, 0.0f);
                GLES20.glClear(GLES20.GL_COLOR_BUFFER_BIT);
                GLES20.glUseProgram(program);
                GLES20.glActiveTexture(GLES20.GL_TEXTURE0);
                GLES20.glBindTexture(GLES20.GL_TEXTURE_2D, texture);
                GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_MIN_FILTER,
                        GLES20.GL_NEAREST);
                GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_MAG_FILTER,
                        GLES20.GL_NEAREST);
                GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_WRAP_S,
                        GLES20.GL_CLAMP_TO_EDGE);
                GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_WRAP_T,
                        GLES20.GL_CLAMP_TO_EDGE);
                GLUtils.texImage2D(GLES20.GL_TEXTURE_2D, 0, bitmap, 0);
                GLES20.glUniform1i(textureSlot, 0);
                GLES20.glEnableVertexAttribArray(positionSlot);
                GLES20.glVertexAttribPointer(positionSlot, 2, GLES20.GL_FLOAT, false, 0,
                        floatBuffer(-1f, -1f, 1f, -1f, -1f, 1f, 1f, 1f));
                GLES20.glEnableVertexAttribArray(uvSlot);
                // Bitmap rows are top-down while GL texture rows are
                // bottom-up, so the top vertices sample v=0. The crop keeps
                // a clipped surface at the full terminal's scale.
                GLES20.glVertexAttribPointer(uvSlot, 2, GLES20.GL_FLOAT, false, 0,
                        floatBuffer(u0, v1, u1, v1, u0, v0, u1, v0));
                GLES20.glDrawArrays(GLES20.GL_TRIANGLE_STRIP, 0, 4);
                GLES20.glDisableVertexAttribArray(positionSlot);
                GLES20.glDisableVertexAttribArray(uvSlot);
                boolean firstFrame = !rendered;
                String sampleText = "";
                if (firstFrame) {
                    java.nio.ByteBuffer sample = ByteBuffer.allocateDirect(4);
                    GLES20.glReadPixels(Math.max(0, getWidth() / 2),
                            Math.max(0, getHeight() / 2),
                            1, 1, GLES20.GL_RGBA, GLES20.GL_UNSIGNED_BYTE, sample);
                    sampleText = (sample.get(0) & 0xff) + "," + (sample.get(1) & 0xff)
                            + "," + (sample.get(2) & 0xff) + "," + (sample.get(3) & 0xff);
                }
                int glError = GLES20.glGetError();
                if (!EGL14.eglSwapBuffers(eglDisplay, eglSurface)) {
                    releaseGl();
                    return;
                }
                if (firstFrame) {
                    rendered = true;
                    trace("vtScene", "gl", "frame", "page", pageIndex,
                            "w", width, "h", height, "rgba", sampleText,
                            "surface", getWidth() + "x" + getHeight(),
                            "error", glError);
                    if (repaintRequested) {
                        repaintRequested = false;
                        trace("vtScene", "gl", "repaint", "page", pageIndex,
                                "reason", repaintReason,
                                "surface", getWidth() + "x" + getHeight());
                    }
                }
            } finally {
                EGL14.eglMakeCurrent(eglDisplay, EGL14.EGL_NO_SURFACE,
                        EGL14.EGL_NO_SURFACE, EGL14.EGL_NO_CONTEXT);
            }
        }

        private java.nio.FloatBuffer floatBuffer(float... values) {
            java.nio.FloatBuffer buffer = ByteBuffer.allocateDirect(values.length * 4)
                    .order(java.nio.ByteOrder.nativeOrder()).asFloatBuffer();
            buffer.put(values).position(0);
            return buffer;
        }

        private void releaseGl() {
            glReady = false;
            rendered = false;
            if (eglDisplay != EGL14.EGL_NO_DISPLAY) {
                if (eglSurface != EGL14.EGL_NO_SURFACE
                        && eglContext != EGL14.EGL_NO_CONTEXT) {
                    EGL14.eglMakeCurrent(eglDisplay, eglSurface, eglSurface, eglContext);
                }
                if (program != 0) GLES20.glDeleteProgram(program);
                if (texture != 0) GLES20.glDeleteTextures(1, new int[] {texture}, 0);
                EGL14.eglMakeCurrent(eglDisplay, EGL14.EGL_NO_SURFACE,
                        EGL14.EGL_NO_SURFACE, EGL14.EGL_NO_CONTEXT);
                if (eglSurface != EGL14.EGL_NO_SURFACE) {
                    EGL14.eglDestroySurface(eglDisplay, eglSurface);
                }
                if (eglContext != EGL14.EGL_NO_CONTEXT) {
                    EGL14.eglDestroyContext(eglDisplay, eglContext);
                }
                EGL14.eglTerminate(eglDisplay);
            }
            program = 0;
            texture = 0;
            eglSurface = EGL14.EGL_NO_SURFACE;
            eglContext = EGL14.EGL_NO_CONTEXT;
            eglDisplay = EGL14.EGL_NO_DISPLAY;
            eglConfig = null;
            bufferWidth = 0;
            bufferHeight = 0;
        }
    }

    private final class RenderedPage extends FrameLayout {
        final int index;
        Bitmap raster;
        final Paint rasterPaint = new Paint(Paint.FILTER_BITMAP_FLAG);
        final android.graphics.Rect rasterBounds = new android.graphics.Rect();
        JSONArray hits = new JSONArray();
        TextView caption;
        double textTop = -1;
        double widthPt = PAGE_WIDTH_PT;
        double heightPt = 841.89;
        boolean hasFigures;
        JSONArray figures = new JSONArray();
        NativeTerminal[] terminalScenes;
        TerminalSurface[] terminalSurfaces;
        int terminalFocus = -1;
        int[] terminalGridWidths;
        int[] terminalGridHeights;
        /** True while this viewer only follows output and cannot input. */
        boolean[] terminalWatchOnly;
        /** A conflicting control lease; input stays blocked until resolved. */
        boolean[] terminalConflict;
        int accessibilityFocusFigure = -1;
        static final int FIGURE_NODE_BASE = 100000;
        // Resource-style custom action ids; the low 0x0102xxxx range belongs
        // to framework actions such as NEXT_HTML_ELEMENT.
        static final int ACTION_FIGURE_GO = 0x7f0f0001;
        static final int ACTION_FIGURE_REBIND = 0x7f0f0002;
        static final int ACTION_FIGURE_DETACH = 0x7f0f0003;
        static final int ACTION_FIGURE_SESSIONS = 0x7f0f0004;
        static final int ACTION_FIGURE_TERMINAL_FOCUS = 0x7f0f0005;
        static final int ACTION_FIGURE_TERMINAL_COPY = 0x7f0f0006;
        static final int ACTION_FIGURE_TERMINAL_PASTE = 0x7f0f0007;
        boolean loaded;
        long bitmapBytes;
        long lastVisible;
        final Paint caretPaint = new Paint(Paint.ANTI_ALIAS_FLAG);
        RenderedPage(android.content.Context context, int index) {
            super(context);
            this.index = index;
            setWillNotDraw(false);
            caretPaint.setColor(Color.parseColor("#B3CFE4"));
            caretPaint.setStyle(Paint.Style.STROKE);
            caretPaint.setStrokeWidth(Math.max(2f, getResources().getDisplayMetrics().density));
            setFocusable(true);
            setFocusableInTouchMode(true);
            setClipChildren(true);
        }
        float pointScale() {
            return (float) (getWidth() / Math.max(1.0, widthPt));
        }

        /** Screen-space point for the start (0) or end (1) selection handle. */
        private float[] selectionHandlePoint(int which) {
            if (selectionAnchor < 0 || editor == null
                    || selectionAnchor == editor.getSelectionStart()) return null;
            int begin = Math.min(selectionAnchor, editor.getSelectionStart());
            int end = Math.max(selectionAnchor, editor.getSelectionStart());
            JSONArray chosen = null;
            int chosenPos = which == 0 ? Integer.MAX_VALUE : -1;
            for (int i = 0; i < hits.length(); i++) {
                JSONArray hit = hits.optJSONArray(i);
                if (hit == null) continue;
                int pos = hit.optInt(4, -1);
                if (pos < begin || pos >= end) continue;
                if (which == 0 ? pos < chosenPos : pos > chosenPos) {
                    chosen = hit;
                    chosenPos = pos;
                }
            }
            if (chosen == null) return null;
            float factor = pointScale();
            float x = (float) (which == 0 ? chosen.optDouble(0) : chosen.optDouble(2)) * factor;
            float bottom = (float) chosen.optDouble(3) * factor;
            return new float[] {x, bottom};
        }

        /** Which handle is under a local touch, or -1. */
        int selectionHandleAt(float x, float y) {
            float[] start = selectionHandlePoint(0);
            float[] end = selectionHandlePoint(1);
            float radius = dp(24);
            if (start != null && Math.hypot(x - start[0], y - start[1] - dp(8)) <= radius) return 0;
            if (end != null && Math.hypot(x - end[0], y - end[1] - dp(8)) <= radius) return 1;
            return -1;
        }

        private void drawSelectionHandles(Canvas canvas) {
            Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
            paint.setColor(Color.parseColor("#B3CFE4"));
            paint.setStyle(Paint.Style.FILL);
            float radius = dp(7);
            for (int which = 0; which < 2; which++) {
                float[] point = selectionHandlePoint(which);
                if (point == null) continue;
                float y = point[1] + dp(8);
                canvas.drawRect(point[0] - dp(1), point[1], point[0] + dp(1), y, paint);
                canvas.drawCircle(point[0], y, radius, paint);
            }
        }

        private final android.view.accessibility.AccessibilityNodeProvider provider =
                new android.view.accessibility.AccessibilityNodeProvider() {
            @Override public android.view.accessibility.AccessibilityNodeInfo
                    createAccessibilityNodeInfo(int virtualViewId) {
                if (virtualViewId == HOST_VIEW_ID) return hostNode();
                List<AccessibleBlock> blocks = accessibleBlocks(RenderedPage.this);
                if (virtualViewId >= 0 && virtualViewId < blocks.size()) {
                    return blockNode(virtualViewId, blocks.get(virtualViewId));
                }
                int figureIndex = virtualViewId - FIGURE_NODE_BASE;
                if (loaded && figureIndex >= 0 && figureIndex < figures.length()) {
                    return figureNode(figureIndex, figures.optJSONObject(figureIndex));
                }
                return null;
            }
            @Override public boolean performAction(int virtualViewId, int action,
                    android.os.Bundle arguments) {
                if (!loaded) return false;
                if (virtualViewId == HOST_VIEW_ID) return false;
                List<AccessibleBlock> blocks = accessibleBlocks(RenderedPage.this);
                if (virtualViewId >= 0 && virtualViewId < blocks.size()) {
                    AccessibleBlock block = blocks.get(virtualViewId);
                    if (action == android.view.accessibility.AccessibilityNodeInfo.ACTION_CLICK) {
                        activateAccessibleBlock(block);
                        sendAccessibilityNodeEvent(virtualViewId,
                                android.view.accessibility.AccessibilityEvent.TYPE_VIEW_CLICKED, block);
                        return true;
                    }
                    if (action == android.view.accessibility.AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS) {
                        accessibilityFocusOffset = block.start;
                        accessibilityFocusFigure = -1;
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
                int figureIndex = virtualViewId - FIGURE_NODE_BASE;
                if (figureIndex < 0 || figureIndex >= figures.length()) return false;
                JSONObject figure = figures.optJSONObject(figureIndex);
                if (figure == null) return false;
                String reference = jsonString(figure, "reference");
                SessionCatalog.Entry entry = reference.isEmpty()
                        ? null : sessionCatalog.byReference(reference);
                if (action == android.view.accessibility.AccessibilityNodeInfo.ACTION_CLICK
                        || action == ACTION_FIGURE_GO) {
                    if (!reference.isEmpty() && jsonString(figure, "bindingError").isEmpty()) {
                        goToFigure(RenderedPage.this, figure);
                    } else {
                        showSessions();
                    }
                    sendFigureEvent(virtualViewId,
                            android.view.accessibility.AccessibilityEvent.TYPE_VIEW_CLICKED,
                            figureDescription(figure));
                    return true;
                }
                if (action == ACTION_FIGURE_REBIND && entry != null) {
                    showRebindDialog(entry);
                    return true;
                }
                if (action == ACTION_FIGURE_DETACH && entry != null && entry.canDetach()) {
                    sessionCatalog = sessionCatalog.detach(entry.reference);
                    invalidate();
                    sendFigureEvent(virtualViewId,
                            android.view.accessibility.AccessibilityEvent.TYPE_VIEW_CLICKED,
                            entry.caption + ". Detached. The session stays on its host.");
                    return true;
                }
                if (action == ACTION_FIGURE_SESSIONS) {
                    showSessions();
                    return true;
                }
                boolean hasTerminal = terminalScenes != null
                        && figureIndex < terminalScenes.length
                        && terminalScenes[figureIndex] != null;
                if (hasTerminal && action == ACTION_FIGURE_TERMINAL_FOCUS) {
                    focusTerminal(RenderedPage.this, figureIndex);
                    sendFigureEvent(virtualViewId,
                            android.view.accessibility.AccessibilityEvent.TYPE_VIEW_CLICKED,
                            "Terminal focused. Type to send input.");
                    return true;
                }
                if (hasTerminal && action == ACTION_FIGURE_TERMINAL_COPY) {
                    copyTerminalOutput(RenderedPage.this, figureIndex);
                    sendFigureEvent(virtualViewId,
                            android.view.accessibility.AccessibilityEvent.TYPE_VIEW_CLICKED,
                            "Terminal output copied.");
                    return true;
                }
                if (hasTerminal && action == ACTION_FIGURE_TERMINAL_PASTE) {
                    focusTerminal(RenderedPage.this, figureIndex);
                    pasteTerminalClipboard();
                    sendFigureEvent(virtualViewId,
                            android.view.accessibility.AccessibilityEvent.TYPE_VIEW_CLICKED,
                            "Pasted into terminal.");
                    return true;
                }
                if (action == android.view.accessibility.AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS) {
                    accessibilityFocusFigure = figureIndex;
                    accessibilityFocusOffset = -1;
                    invalidate();
                    sendFigureEvent(virtualViewId,
                            android.view.accessibility.AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUSED,
                            figureDescription(figure));
                    return true;
                }
                if (action == android.view.accessibility.AccessibilityNodeInfo.ACTION_CLEAR_ACCESSIBILITY_FOCUS) {
                    if (accessibilityFocusFigure == figureIndex) accessibilityFocusFigure = -1;
                    invalidate();
                    sendFigureEvent(virtualViewId,
                            android.view.accessibility.AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED,
                            figureDescription(figure));
                    return true;
                }
                return false;
            }
            @Override public android.view.accessibility.AccessibilityNodeInfo findFocus(int focus) {
                if (focus != android.view.accessibility.AccessibilityNodeInfo.FOCUS_ACCESSIBILITY) {
                    return null;
                }
                if (accessibilityFocusFigure >= 0 && accessibilityFocusFigure < figures.length()) {
                    return figureNode(accessibilityFocusFigure,
                            figures.optJSONObject(accessibilityFocusFigure));
                }
                if (accessibilityFocusOffset < 0) return null;
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
                for (int i = 0; i < figures.length(); i++) {
                    info.addChild(RenderedPage.this, FIGURE_NODE_BASE + i);
                }
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

        private SessionCatalog.Entry figureEntry(JSONObject figure) {
            if (figure == null) return null;
            String reference = jsonString(figure, "reference");
            return reference.isEmpty() ? null : sessionCatalog.byReference(reference);
        }

        private String figureDescription(JSONObject figure) {
            return SessionCatalog.figureDescription(figureEntry(figure),
                    jsonString(figure, "reference"),
                    jsonString(figure, "bindingError"));
        }

        private android.view.accessibility.AccessibilityNodeInfo figureNode(
                int index, JSONObject figure) {
            if (figure == null) return null;
            android.view.accessibility.AccessibilityNodeInfo info =
                    new android.view.accessibility.AccessibilityNodeInfo();
            int id = FIGURE_NODE_BASE + index;
            info.setSource(RenderedPage.this, id);
            info.setParent(RenderedPage.this);
            info.setPackageName(getPackageName());
            info.setClassName("android.view.View");
            String description = figureDescription(figure);
            if (terminalScenes != null && index < terminalScenes.length
                    && terminalScenes[index] != null && terminalWatchOnly != null
                    && index < terminalWatchOnly.length && terminalWatchOnly[index]) {
                description += " Read-only; take control to type.";
            }
            info.setContentDescription(description);
            info.setEnabled(true);
            info.setFocusable(true);
            info.setClickable(true);
            float scale = pointScale();
            int[] location = new int[2];
            getLocationOnScreen(location);
            android.graphics.Rect local = new android.graphics.Rect(
                    Math.round((float) figure.optDouble("x", 0) * scale),
                    Math.round((float) figure.optDouble("y", 0) * scale),
                    Math.round((float) (figure.optDouble("x", 0)
                            + figure.optDouble("w", 0)) * scale),
                    Math.round((float) (figure.optDouble("y", 0)
                            + figure.optDouble("h", 0)) * scale));
            info.setBoundsInScreen(new android.graphics.Rect(local.left + location[0],
                    local.top + location[1], local.right + location[0],
                    local.bottom + location[1]));
            android.graphics.Rect visible = new android.graphics.Rect();
            info.setVisibleToUser(pageVisibleToUser()
                    && getLocalVisibleRect(visible) && android.graphics.Rect.intersects(visible, local));
            boolean focused = accessibilityFocusFigure == index;
            info.setAccessibilityFocused(focused);
            info.addAction(android.view.accessibility.AccessibilityNodeInfo.AccessibilityAction.ACTION_CLICK);
            info.addAction(new android.view.accessibility.AccessibilityNodeInfo.AccessibilityAction(
                    ACTION_FIGURE_GO, "Go to figure"));
            SessionCatalog.Entry entry = figureEntry(figure);
            if (entry != null) {
                info.addAction(new android.view.accessibility.AccessibilityNodeInfo.AccessibilityAction(
                        ACTION_FIGURE_REBIND, SessionCatalog.actionLabel("REBIND", entry)));
                if (entry.canDetach()) {
                    info.addAction(new android.view.accessibility.AccessibilityNodeInfo.AccessibilityAction(
                            ACTION_FIGURE_DETACH, SessionCatalog.actionLabel("DETACH", entry)));
                }
            }
            info.addAction(new android.view.accessibility.AccessibilityNodeInfo.AccessibilityAction(
                    ACTION_FIGURE_SESSIONS, "Sessions"));
            if (terminalScenes != null && index < terminalScenes.length
                    && terminalScenes[index] != null) {
                info.addAction(new android.view.accessibility.AccessibilityNodeInfo.AccessibilityAction(
                        ACTION_FIGURE_TERMINAL_FOCUS, "Focus terminal"));
                info.addAction(new android.view.accessibility.AccessibilityNodeInfo.AccessibilityAction(
                        ACTION_FIGURE_TERMINAL_COPY, "Copy terminal output"));
                info.addAction(new android.view.accessibility.AccessibilityNodeInfo.AccessibilityAction(
                        ACTION_FIGURE_TERMINAL_PASTE, "Paste into terminal"));
            }
            info.addAction(focused
                    ? android.view.accessibility.AccessibilityNodeInfo.AccessibilityAction.ACTION_CLEAR_ACCESSIBILITY_FOCUS
                    : android.view.accessibility.AccessibilityNodeInfo.AccessibilityAction.ACTION_ACCESSIBILITY_FOCUS);
            return info;
        }

        private void sendFigureEvent(int id, int type, String description) {
            // Never make an accessibility action wait on the accessibility
            // service; without an enabled service this call can block.
            if (!accessibilityEnabled()) return;
            post(new Runnable() {
                @Override public void run() {
                    if (!accessibilityEnabled()) return;
                    android.view.accessibility.AccessibilityEvent event =
                            new android.view.accessibility.AccessibilityEvent();
                    event.setEventType(type);
                    event.setSource(RenderedPage.this, id);
                    event.setPackageName(getPackageName());
                    event.setClassName("android.view.View");
                    event.getText().add(description);
                    if (getParent() != null) {
                        getParent().requestSendAccessibilityEvent(RenderedPage.this, event);
                    }
                }
            });
        }

        /** Native scene rectangle in page pixels, or null for a non-terminal slot. */
        private android.graphics.RectF terminalRect(int index) {
            JSONObject figure = figures.optJSONObject(index);
            if (figure == null || jsonString(figure, "reference").isEmpty()) return null;
            if (raster == null || getWidth() <= 0 || widthPt <= 0) return null;
            float scale = pointScale();
            float x = (float) figure.optDouble("x", 0) * scale;
            float y = (float) figure.optDouble("y", 0) * scale;
            float w = (float) figure.optDouble("w", 0) * scale;
            float h = (float) figure.optDouble("h", 0) * scale;
            if (w <= 0 || h <= 0) return null;
            return new android.graphics.RectF(x, y, x + w, y + h);
        }

        int terminalIndexAt(float x, float y) {
            if (terminalScenes == null) return -1;
            for (int i = 0; i < terminalScenes.length; i++) {
                if (terminalScenes[i] == null) continue;
                android.graphics.RectF rect = terminalRect(i);
                if (rect != null && rect.contains(x, y)) return i;
            }
            return -1;
        }

        /** Cell under a page-local point in a terminal scene, or null. */
        int[] terminalCellAt(int index, float x, float y) {
            if (terminalScenes == null || index < 0 || index >= terminalScenes.length
                    || terminalScenes[index] == null) return null;
            android.graphics.RectF rect = terminalRect(index);
            if (rect == null || rect.isEmpty()) return null;
            NativeTerminal scene = terminalScenes[index];
            int cols = Math.max(1, scene.cols());
            int rows = Math.max(1, scene.rows());
            int col = (int) ((x - rect.left) / Math.max(1f, rect.width()) * cols);
            int row = (int) ((y - rect.top) / Math.max(1f, rect.height()) * rows);
            return new int[] {Math.max(0, Math.min(cols - 1, col)),
                    Math.max(0, Math.min(rows - 1, row))};
        }

        /**
         * Reflow a scene's grid after the page width changed (view zoom).
         * The fixture's manual resize does not move the page rect, so this
         * only runs when the placed rectangle actually changed.
         */
        private void syncTerminalGrid(int index, NativeTerminal scene, android.graphics.RectF rect) {
            if (terminalGridWidths == null || terminalGridHeights == null
                    || index >= terminalGridWidths.length) return;
            int width = Math.round(rect.width());
            int height = Math.round(rect.height());
            if (terminalGridWidths[index] <= 0 || terminalGridHeights[index] <= 0) {
                terminalGridWidths[index] = width;
                terminalGridHeights[index] = height;
                return;
            }
            if (Math.abs(width - terminalGridWidths[index]) <= 1
                    && Math.abs(height - terminalGridHeights[index]) <= 1) return;
            terminalGridWidths[index] = width;
            terminalGridHeights[index] = height;
            int cols = Math.max(20, Math.min(300, width / TERMINAL_CELL_WIDTH));
            int rows = Math.max(5, Math.min(120, height / TERMINAL_CELL_HEIGHT));
            if (cols != scene.cols() || rows != scene.rows()) {
                scene.resize(cols, rows);
                trace("vtScene", "phase", "reflow", "page", RenderedPage.this.index,
                        "index", index, "cols", cols, "rows", rows,
                        "w", width, "h", height);
            }
        }

        /** Rectangle covering an inclusive cell range in the scene. */
        android.graphics.RectF terminalCellRect(int index, int x0, int y0, int x1, int y1) {
            android.graphics.RectF rect = terminalRect(index);
            if (rect == null || terminalScenes == null || index < 0
                    || index >= terminalScenes.length || terminalScenes[index] == null) return null;
            NativeTerminal scene = terminalScenes[index];
            int cols = Math.max(1, scene.cols());
            int rows = Math.max(1, scene.rows());
            int left = Math.min(x0, x1), right = Math.max(x0, x1) + 1;
            int top = Math.min(y0, y1), bottom = Math.max(y0, y1) + 1;
            float cw = rect.width() / cols;
            float ch = rect.height() / rows;
            return new android.graphics.RectF(rect.left + left * cw, rect.top + top * ch,
                    rect.left + Math.min(cols, right) * cw,
                    rect.top + Math.min(rows, bottom) * ch);
        }

        void closeTerminalScenes() {
            if (terminalSelectionPage == this) clearTerminalSelection();
            if (terminalMousePage == this) {
                terminalMousePage = null;
                terminalMouseIndex = -1;
                terminalMousePressed = false;
            }
            if (terminalScenes != null) {
                for (int i = 0; i < terminalScenes.length; i++) {
                    if (terminalScenes[i] != null) {
                        terminalScenes[i].close();
                        terminalScenes[i] = null;
                    }
                }
            }
            releaseTerminalSurfaces();
            terminalWatchOnly = null;
            terminalConflict = null;
            terminalFocus = -1;
        }

        private void releaseTerminalSurfaces() {
            if (terminalSurfaces != null) {
                for (int i = 0; i < terminalSurfaces.length; i++) {
                    TerminalSurface surface = terminalSurfaces[i];
                    if (surface == null) continue;
                    surface.release();
                    removeView(surface);
                    terminalSurfaces[i] = null;
                }
            }
            terminalSurfaces = null;
        }

        /** Position one GL surface over its figure rectangle inside the page. */
        private TerminalSurface syncTerminalSurface(int index, NativeTerminal scene,
                android.graphics.RectF rect) {
            if (terminalSurfaces == null || index >= terminalSurfaces.length) {
                int size = terminalScenes == null ? index + 1 : terminalScenes.length;
                terminalSurfaces = new TerminalSurface[Math.max(size, index + 1)];
            }
            TerminalSurface surface = terminalSurfaces[index];
            if (surface == null) {
                surface = new TerminalSurface(getContext(), RenderedPage.this.index, scene);
                terminalSurfaces[index] = surface;
                addView(surface);
            }
            // A z-ordered SurfaceView is not clipped by its parents. Show
            // only the part of the figure that intersects this page and the
            // preview viewport, and crop the texture to the same fraction.
            android.graphics.RectF visible = new android.graphics.RectF(
                    Math.max(0f, rect.left), Math.max(0f, rect.top),
                    Math.min(getWidth(), rect.right), Math.min(getHeight(), rect.bottom));
            // Screen locations can lag one layout pass behind a scrollTo.
            // The page sits in the previewBox content, so its own left/top
            // give a stable page-local viewport from the scroll offsets.
            float viewportLeft = previewHScroll.getScrollX() - getLeft();
            float viewportTop = previewScroll.getScrollY() - getTop();
            android.graphics.RectF viewport = new android.graphics.RectF(
                    viewportLeft, viewportTop,
                    viewportLeft + previewScroll.getWidth(),
                    viewportTop + previewScroll.getHeight());
            if (visible.width() <= 0 || visible.height() <= 0
                    || !visible.intersect(viewport)) {
                surface.setVisibility(View.GONE);
                return surface;
            }
            boolean changed = surface.getVisibility() != View.VISIBLE;
            surface.setVisibility(View.VISIBLE);
            surface.setCrop((visible.left - rect.left) / Math.max(1f, rect.width()),
                    (visible.top - rect.top) / Math.max(1f, rect.height()),
                    (visible.right - rect.left) / Math.max(1f, rect.width()),
                    (visible.bottom - rect.top) / Math.max(1f, rect.height()));
            int left = Math.round(visible.left);
            int top = Math.round(visible.top);
            int width = Math.max(1, Math.round(visible.width()));
            int height = Math.max(1, Math.round(visible.height()));
            FrameLayout.LayoutParams params;
            android.view.ViewGroup.LayoutParams current = surface.getLayoutParams();
            if (current instanceof FrameLayout.LayoutParams) {
                params = (FrameLayout.LayoutParams) current;
            } else {
                params = new FrameLayout.LayoutParams(width, height);
                surface.setLayoutParams(params);
                changed = true;
            }
            if (params.leftMargin != left || params.topMargin != top
                    || params.width != width || params.height != height) {
                params.leftMargin = left;
                params.topMargin = top;
                params.width = width;
                params.height = height;
                surface.setLayoutParams(params);
                changed = true;
            }
            if (changed) surface.repaint("layout");
            return surface;
        }

        void recreateTerminalSurface(int index) {
            if (terminalSurfaces == null || index < 0 || index >= terminalSurfaces.length
                    || terminalSurfaces[index] == null) return;
            terminalSurfaces[index].recreateContext();
        }

        /** Re-clip z-ordered scene surfaces after a preview scroll. */
        void clipTerminalSurfaces() {
            if (terminalScenes == null) return;
            for (int i = 0; i < terminalScenes.length; i++) {
                NativeTerminal scene = terminalScenes[i];
                if (scene == null) continue;
                android.graphics.RectF rect = terminalRect(i);
                if (rect != null) syncTerminalSurface(i, scene, rect);
            }
        }

        private void drawTerminalScenes(Canvas canvas) {
            if (terminalScenes == null) return;
            Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
            paint.setFilterBitmap(false);
            for (int i = 0; i < terminalScenes.length; i++) {
                NativeTerminal scene = terminalScenes[i];
                if (scene == null) continue;
                android.graphics.RectF rect = terminalRect(i);
                if (rect == null) continue;
                syncTerminalGrid(i, scene, rect);
                TerminalSurface surface = syncTerminalSurface(i, scene, rect);
                Paint fill = new Paint();
                fill.setColor(Color.parseColor("#14181D"));
                canvas.drawRect(rect, fill);
                if (surface == null || !surface.submitting()) {
                    Bitmap bitmap = scene.bitmap();
                    if (bitmap != null) {
                        canvas.save();
                        canvas.clipRect(rect);
                        canvas.drawBitmap(bitmap, null, rect, paint);
                        canvas.restore();
                    }
                }
                if (surface != null) surface.requestFrame();
                if (terminalSelectionPage == RenderedPage.this
                        && terminalSelectionIndex == i) {
                    android.graphics.RectF selected = terminalCellRect(i,
                            terminalSelX0, terminalSelY0, terminalSelX1, terminalSelY1);
                    if (selected != null) {
                        Paint highlight = new Paint();
                        highlight.setColor(0x66628AB3);
                        canvas.drawRect(selected, highlight);
                    }
                }
                boolean watchOnly = terminalWatchOnly != null
                        && i < terminalWatchOnly.length && terminalWatchOnly[i];
                boolean conflict = terminalConflict != null
                        && i < terminalConflict.length && terminalConflict[i];
                if (watchOnly || conflict) {
                    Paint badge = new Paint(Paint.ANTI_ALIAS_FLAG);
                    badge.setColor(conflict ? 0xCCB4633A : 0xCC2E6E8E);
                    float right = Math.min(rect.right, getWidth()) - dp(4);
                    float left = right - dp(conflict ? 68 : 44);
                    float top = rect.top + dp(4);
                    canvas.drawRect(left, top, right, top + dp(16), badge);
                    Paint label = new Paint(Paint.ANTI_ALIAS_FLAG);
                    label.setColor(0xFFF2F6FA);
                    label.setTextSize(dp(10));
                    canvas.drawText(conflict ? "CONFLICT" : "WATCH",
                            left + dp(4), top + dp(12), label);
                }
                if (terminalFocus == i) {
                    Paint border = new Paint(Paint.ANTI_ALIAS_FLAG);
                    border.setStyle(Paint.Style.STROKE);
                    border.setStrokeWidth(dp(2));
                    border.setColor(Color.parseColor("#B3CFE4"));
                    canvas.drawRect(rect, border);
                }
            }
        }

        /** The page owns terminal gestures; scene surfaces never consume them. */
        @Override public boolean onInterceptTouchEvent(MotionEvent event) {
            return true;
        }

        @Override public boolean onCheckIsTextEditor() {
            return terminalFocus >= 0;
        }

        /** Back releases terminal focus before the IME consumes it. */
        @Override public boolean onKeyPreIme(int keyCode, KeyEvent event) {
            if (keyCode == KeyEvent.KEYCODE_BACK && event.getAction() == KeyEvent.ACTION_DOWN
                    && terminalFocus >= 0) {
                releaseTerminalFocus();
                return true;
            }
            return super.onKeyPreIme(keyCode, event);
        }

        @Override public android.view.inputmethod.InputConnection onCreateInputConnection(
                android.view.inputmethod.EditorInfo out) {
            if (terminalFocus < 0) return null;
            out.inputType = android.text.InputType.TYPE_CLASS_TEXT
                    | android.text.InputType.TYPE_TEXT_FLAG_MULTI_LINE
                    | android.text.InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS;
            out.imeOptions = android.view.inputmethod.EditorInfo.IME_ACTION_NONE;
            return new android.view.inputmethod.BaseInputConnection(this, true) {
                @Override public boolean setComposingText(CharSequence text, int newCursorPosition) {
                    terminalComposing = text == null ? "" : text.toString();
                    return true;
                }
                @Override public boolean finishComposingText() {
                    String text = terminalComposing;
                    terminalComposing = "";
                    sendTerminalText(text);
                    return true;
                }
                @Override public boolean commitText(CharSequence text, int newCursorPosition) {
                    terminalComposing = "";
                    sendTerminalText(text == null ? "" : text.toString());
                    return true;
                }
                @Override public boolean deleteSurroundingText(int beforeLength, int afterLength) {
                    for (int i = 0; i < beforeLength; i++) sendTerminalKey(KeyEvent.KEYCODE_DEL, 0, "", 0);
                    return true;
                }
            };
        }

        /** Draw the public session state over each terminal figure slot. */
        private void drawFigureStates(Canvas canvas) {
            if (figures == null || figures.length() == 0) return;
            float scale = pointScale();
            Paint background = new Paint(Paint.ANTI_ALIAS_FLAG);
            Paint label = new Paint(Paint.ANTI_ALIAS_FLAG);
            label.setTextSize(dp(10));
            for (int i = 0; i < figures.length(); i++) {
                JSONObject figure = figures.optJSONObject(i);
                if (figure == null) continue;
                String text;
                int color;
                String bindingError = jsonString(figure, "bindingError");
                String reference = jsonString(figure, "reference");
                if (!bindingError.isEmpty()) {
                    text = "Invalid binding";
                    color = 0xCCB4633A;
                } else if (!reference.isEmpty()) {
                    SessionCatalog.Entry entry = sessionCatalog.byReference(reference);
                    if (entry == null) {
                        text = "Unavailable";
                        color = 0xCC6E7781;
                    } else {
                        if (entry.state == SessionCatalog.State.LIVE) continue;
                        text = entry.stateLabel();
                        color = entry.state == SessionCatalog.State.WATCHED
                                ? 0xCC2E6E8E : 0xCC6E7781;
                    }
                } else {
                    continue;
                }
                float x = (float) figure.optDouble("x", 0) * scale;
                float y = (float) figure.optDouble("y", 0) * scale;
                float w = (float) figure.optDouble("w", 0) * scale;
                float h = (float) figure.optDouble("h", 0) * scale;
                background.setColor(color);
                canvas.drawRect(x, y + h - dp(18), x + w, y + h, background);
                label.setColor(0xFFF2F6FA);
                canvas.drawText(text, x + dp(5), y + h - dp(5), label);
            }
        }

        private boolean accessibilityEnabled() {
            android.view.accessibility.AccessibilityManager manager =
                    (android.view.accessibility.AccessibilityManager)
                            getSystemService(android.content.Context.ACCESSIBILITY_SERVICE);
            return manager != null && manager.isEnabled();
        }

        private void sendAccessibilityNodeEvent(int id, int type, AccessibleBlock block) {
            if (!accessibilityEnabled()) return;
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
            if (raster != null) {
                // The raster can be smaller than the view under the pixel
                // budget. Use the page bounds shared by the Typst hit map.
                rasterBounds.set(0, 0, getWidth(), getHeight());
                canvas.drawBitmap(raster, null, rasterBounds, rasterPaint);
            }
        }

        @Override protected void dispatchDraw(Canvas canvas) {
            super.dispatchDraw(canvas);
            drawTerminalScenes(canvas);
            drawPageOverlays(canvas);
            drawFigureStates(canvas);
        }

        /** Selection, find hits and the caret stay above the GL surface. */
        private void drawPageOverlays(Canvas canvas) {
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
                drawSelectionHandles(canvas);
            }
            if (searchBar != null && searchBar.getVisibility() == View.VISIBLE) {
                Paint highlight = new Paint();
                float factor = pointScale();
                for (int i = 0; i < hits.length(); i++) {
                    JSONArray hit = hits.optJSONArray(i);
                    if (hit == null) continue;
                    int pos = hit.optInt(4, -1);
                    if (find == null) continue;
                    FindController.Match[] matches = find.pageMatches();
                    int lo = 0, hi = matches.length;
                    while (lo < hi) {
                        int mid = (lo + hi) >>> 1;
                        if (matches[mid].end <= pos) lo = mid + 1;
                        else hi = mid;
                    }
                    if (lo < matches.length) {
                        FindController.Match match = matches[lo];
                        if (pos >= match.start && pos < match.end) {
                            int global = find.pageStart() + lo;
                            highlight.setColor(global == find.currentGlobal()
                                    ? 0x887DBB55 : 0x66898920);
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

    private boolean findVisible() {
        return searchBar != null && searchBar.getVisibility() == View.VISIBLE;
    }

    private boolean searchTextHasFocus() {
        View focus = getCurrentFocus();
        if (!(focus instanceof EditText)) return false;
        for (View view = focus; view != null; ) {
            if (view == searchBar) return true;
            android.view.ViewParent parent = view.getParent();
            view = parent instanceof View ? (View) parent : null;
        }
        return false;
    }

    private boolean imeActive(View view) {
        if (view == null) return false;
        android.view.inputmethod.InputMethodManager ime =
                (android.view.inputmethod.InputMethodManager) getSystemService(INPUT_METHOD_SERVICE);
        return ime != null && ime.isActive(view);
    }

    private void hideIme(View view) {
        if (view == null) return;
        android.view.inputmethod.InputMethodManager ime =
                (android.view.inputmethod.InputMethodManager) getSystemService(INPUT_METHOD_SERVICE);
        if (ime != null) ime.hideSoftInputFromWindow(view.getWindowToken(), 0);
    }

    /** One implementation for every toolbar button and its advertised key. */
    // ----- native document sharing (K6) ---------------------------------

    private void shareOpen() {
        if (shareHandle != 0) return;
        String peer = "pad-" + android.os.Process.myPid();
        shareHandle = DocBridge.shareOpen(DocBridge.snapshotBytes(handle), peer, peer);
        shareWatchOnly = false;
        // A peer socket supplied by the launch intent is the frame transport:
        // the worker dials it once and carries both directions from there.
        String socket = getIntent() == null
                ? null : getIntent().getStringExtra("emthin.shareSocket");
        if (shareHandle != 0 && socket != null && !socket.isEmpty()) {
            try {
                org.json.JSONObject reply = new org.json.JSONObject(
                        DocBridge.shareConnect(shareHandle, socket));
                if (!reply.optBoolean("ok", false)) {
                    trace("share", "link_failed", reply.optString("error", ""));
                }
            } catch (org.json.JSONException e) {
                trace("share", "link_failed", e.getMessage());
            }
        }
        setStatus(shareHandle != 0 ? "Share ready" : "Share unavailable");
        trace("share", "opened", shareHandle != 0);
    }

    private void shareJoin() {
        if (shareHandle == 0) { setStatus("No share to join"); return; }
        if (shareWatchOnly) { setStatus("Watching — switch to Edit to join"); return; }
        DocBridge.shareJoin(shareHandle, "host", "");
        pollShare();
    }

    private void shareLeave() {
        if (shareHandle == 0) return;
        DocBridge.shareLeave(shareHandle);
        setStatus("Left share");
        trace("share", "left");
    }

    private void shareToggleWatch() {
        shareWatchOnly = !shareWatchOnly;
        setStatus(shareWatchOnly ? "Watching share" : "Editing share");
        trace("share", "watch", shareWatchOnly);
    }

    /** Drain worker events and apply accepted deltas as one grouped edit each. */
    private void pollShare() {
        if (shareHandle == 0) return;
        try {
            org.json.JSONObject reply = new org.json.JSONObject(DocBridge.sharePoll(shareHandle));
            org.json.JSONArray events = reply.optJSONArray("events");
            if (events == null) return;
            for (int i = 0; i < events.length(); i++) {
                org.json.JSONObject event = events.getJSONObject(i);
                String kind = event.optString("kind", "");
                if ("error".equals(kind)) {
                    trace("share", "error", event.optString("message", ""));
                    continue;
                }
                if ("outbound".equals(kind)) {
                    // With a socket link the worker writes the frame itself; a
                    // frame that surfaces here has no attached transport, so
                    // record it rather than dropping it silently.
                    trace("share", "outbound", event.optString("peer", "")
                            + " " + event.optLong("seq", 0));
                    continue;
                }
                if (shareWatchOnly) continue;
                if ("catch_up".equals(kind) || "applied".equals(kind)) {
                    org.json.JSONArray deltas = "catch_up".equals(kind)
                            ? new org.json.JSONArray().put(event.optString("delta", ""))
                            : event.optJSONArray("deltas");
                    if (deltas == null) continue;
                    for (int d = 0; d < deltas.length(); d++) {
                        String delta = deltas.optString(d, "");
                        if (!delta.isEmpty()) applyState(DocBridge.shareApply(handle, delta), true);
                    }
                }
            }
        } catch (org.json.JSONException e) {
            trace("share", "malformed_event", e.getMessage());
        }
    }

    private boolean runAction(ActionCatalog.Id action) {
        switch (action) {
            case NEW: confirmNewDocument(); break;
            case UNDO: applyUndo(); break;
            case REDO: applyState(DocBridge.redo(handle), true); break;
            case OPEN: chooseDocument(); break;
            case SAVE: saveToDocument(); break;
            case SAVE_COPY: saveCopyToDocument(); break;
            case EDIT_SOURCE:
                if (handle == 0) return false;
                openSourceAt(editor.getSelectionStart(), editor.getSelectionEnd());
                EditText source = sourceField;
                if (source != null) source.post(() -> {
                    if (source != sourceField || !source.hasFocus()) return;
                    android.view.inputmethod.InputMethodManager ime =
                            getSystemService(android.view.inputmethod.InputMethodManager.class);
                    if (ime != null) ime.showSoftInput(source,
                            android.view.inputmethod.InputMethodManager.SHOW_IMPLICIT);
                });
                break;
            case FIND: openSearch(); break;
            case FIND_REPLACE:
                openSearch();
                if (!replaceVisible) toggleReplaceRow();
                break;
            case FIND_NEXT: moveFind(1); break;
            case FIND_PREVIOUS: moveFind(-1); break;
            case NAVIGATE_LAST: showNavigate(lastNavigateTab); break;
            case NAVIGATE_REFERENCES: showNavigate(NAV_REFERENCES); break;
            case NAVIGATE_SECTIONS: showNavigate(NAV_SECTIONS); break;
            case NAV_BACK: navigateBack(); break;
            case NAV_FORWARD: navigateForward(); break;
            case FOLD: toggleFoldAt(editor.getSelectionStart(), false); break;
            case MARKERS: toggleMarkers(); break;
            case READING_SMALLER: changeReadingSize(-1f); break;
            case READING_LARGER: changeReadingSize(1f); break;
            case ZOOM_OUT: changeViewZoom(-0.5f); break;
            case ZOOM_IN: changeViewZoom(0.5f); break;
            case CLOSE_INSPECTOR: closeInspector(); break;
            case SESSIONS: showSessions(); break;
            case PAIR: showPairing(); break;
            case SHARE_OPEN: shareOpen(); break;
            case SHARE_JOIN: shareJoin(); break;
            case SHARE_LEAVE: shareLeave(); break;
            case SHARE_WATCH: shareToggleWatch(); break;
            case PROOF: startActivityForResult(
                    new Intent(this, ProofClientActivity.class), REQUEST_PROOF); break;
            case HISTORY: showHistory(); break;
            case DIAGNOSTICS: showDiagnostics(); break;
            default: return false;
        }
        return true;
    }

    @Override
    public boolean dispatchKeyEvent(KeyEvent event) {
        int keyCode = event.getKeyCode();
        boolean enter = keyCode == KeyEvent.KEYCODE_ENTER || keyCode == KeyEvent.KEYCODE_NUMPAD_ENTER;
        if (event.getAction() == KeyEvent.ACTION_DOWN && terminalFocusedPage() != null) {
            if (event.getKeyCode() == KeyEvent.KEYCODE_BACK) {
                releaseTerminalFocus();
                return true;
            }
            if (event.getKeyCode() != KeyEvent.KEYCODE_VOLUME_UP
                    && event.getKeyCode() != KeyEvent.KEYCODE_VOLUME_DOWN) {
                int key = NativeTerminal.ghosttyKey(event.getKeyCode());
                if (key != 0) {
                    String text = event.getUnicodeChar() > 0
                            ? new String(Character.toChars(event.getUnicodeChar())) : "";
                    sendTerminalKey(event.getKeyCode(), NativeTerminal.ghosttyMods(event),
                            text, event.getUnicodeChar(0) & 0xffff);
                    return true;
                }
            }
        }
        if (event.getAction() == KeyEvent.ACTION_DOWN && event.getRepeatCount() == 0
                && event.getKeyCode() == KeyEvent.KEYCODE_BACK) {
            if (findVisible()) {
                View focused = getCurrentFocus();
                if (searchTextHasFocus() && imeActive(focused)) {
                    // Let the first Back dismiss the IME; the next one cancels Find.
                    focused.clearFocus();
                    if (searchField != null) searchField.clearFocus();
                    if (replaceField != null) replaceField.clearFocus();
                    // Use the query field's token, the same target that the
                    // working Close/Escape path uses for this overlay.
                    hideIme(searchField);
                } else {
                    closeSearch(true);
                }
                return true;
            }
            if (inspectorOpen) {
                if (sourceField != null && sourceField.hasFocus() && imeActive(sourceField)) {
                    sourceField.clearFocus();
                    hideIme(sourceField);
                    focusRenderedCaret();
                } else {
                    closeInspector();
                }
                return true;
            }
        }
        if (event.getAction() == KeyEvent.ACTION_DOWN && searchBar != null
                && searchBar.getVisibility() == View.VISIBLE) {
            if (event.getKeyCode() == KeyEvent.KEYCODE_ESCAPE) { closeSearch(true); return true; }
            if (enter) { acceptFind(); return true; }
            if (event.isCtrlPressed() && event.getKeyCode() == KeyEvent.KEYCODE_S)
                return runAction(ActionCatalog.Id.FIND_NEXT);
            if (event.isCtrlPressed() && event.getKeyCode() == KeyEvent.KEYCODE_R)
                return runAction(ActionCatalog.Id.FIND_PREVIOUS);
        }
        if (event.getAction() == KeyEvent.ACTION_DOWN && event.getRepeatCount() == 0
                && event.getKeyCode() == KeyEvent.KEYCODE_F3) {
            if (findVisible())
                runAction(event.isShiftPressed()
                        ? ActionCatalog.Id.FIND_PREVIOUS : ActionCatalog.Id.FIND_NEXT);
            else openSearch(lastFindQuery(), event.isShiftPressed());
            return true;
        }
        if (pagesStale && getCurrentFocus() instanceof RenderedPage) {
            // The old frame remains visible, but its hit/source mapping is for
            // a superseded capture. Reject edits until the replacement chunk
            // is accepted instead of mutating through stale positions.
            return true;
        }
        if (event.getAction() == KeyEvent.ACTION_DOWN && event.isAltPressed()
                && !(getCurrentFocus() instanceof EditText)) {
            if (event.getKeyCode() == KeyEvent.KEYCODE_DPAD_LEFT)
                return runAction(ActionCatalog.Id.NAV_BACK);
            if (event.getKeyCode() == KeyEvent.KEYCODE_DPAD_RIGHT)
                return runAction(ActionCatalog.Id.NAV_FORWARD);
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
            if (getCurrentFocus() instanceof RenderedPage && (enter || event.getUnicodeChar() >= 32)) {
                String typed = enter ? "\n" : new String(Character.toChars(event.getUnicodeChar()));
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
                    case KeyEvent.KEYCODE_N: return runAction(ActionCatalog.Id.NEW);
                    case KeyEvent.KEYCODE_Z: return runAction(ActionCatalog.Id.UNDO);
                    case KeyEvent.KEYCODE_Y: return runAction(ActionCatalog.Id.REDO);
                    case KeyEvent.KEYCODE_O: return runAction(ActionCatalog.Id.OPEN);
                    case KeyEvent.KEYCODE_S:
                        return runAction(ActionCatalog.ctrlS(findVisible(),
                                event.isShiftPressed()));
                    case KeyEvent.KEYCODE_F: return runAction(ActionCatalog.Id.FIND);
                    case KeyEvent.KEYCODE_H: return runAction(ActionCatalog.Id.FIND_REPLACE);
                    case KeyEvent.KEYCODE_R:
                        if (event.isShiftPressed()) return runAction(ActionCatalog.Id.FIND);
                        break;
                    case KeyEvent.KEYCODE_K:
                        return runAction(ActionCatalog.Id.NAVIGATE_REFERENCES);
                    case KeyEvent.KEYCODE_G:
                        return runAction(ActionCatalog.Id.NAVIGATE_SECTIONS);
                    case KeyEvent.KEYCODE_L: return runAction(ActionCatalog.Id.FOLD);
                }
                if (event.getKeyCode() >= KeyEvent.KEYCODE_1 && event.getKeyCode() <= KeyEvent.KEYCODE_9) {
                    showCitations(event.getKeyCode() - KeyEvent.KEYCODE_1 + 1);
                    return true;
                }
            } else if (event.getKeyCode() == KeyEvent.KEYCODE_ESCAPE && inspectorOpen) {
                return runAction(ActionCatalog.Id.CLOSE_INSPECTOR);
            }
        }
        if (event.getKeyCode() == KeyEvent.KEYCODE_TAB && !event.isCtrlPressed()
                && !event.isAltPressed()) {
            if (event.getAction() == KeyEvent.ACTION_DOWN && event.getRepeatCount() == 0) {
                runAction(ActionCatalog.Id.MARKERS);
            }
            return true;
        }
        return super.dispatchKeyEvent(event);
    }

    /** Everything Find must restore when it is cancelled. */
    private static final class SearchOrigin {
        final int caretStart, caretEnd, scrollY;
        final StableAnchor reading;
        final int readingFallback;
        final java.util.LinkedHashMap<String, String[]> folds;
        final boolean inspector;

        SearchOrigin(int caretStart, int caretEnd, int scrollY, StableAnchor reading,
                     int readingFallback, java.util.LinkedHashMap<String, String[]> folds,
                     boolean inspector) {
            this.caretStart = caretStart;
            this.caretEnd = caretEnd;
            this.scrollY = scrollY;
            this.reading = reading;
            this.readingFallback = readingFallback;
            this.folds = folds;
            this.inspector = inspector;
        }
    }

    private void ensureFind() {
        if (find != null) return;
        find = new FindController(new FindController.SearchFn() {
            @Override public FindController.Page search(FindController.Request request) throws Exception {
                String scope = request.scope == FindController.SCOPE_SOURCE ? "source"
                        : request.scope == FindController.SCOPE_SELECTION ? "selection" : "text";
                JSONObject o = new JSONObject(DocBridge.searchScoped(request.text, request.query,
                        scope, request.selectionStart, request.selectionEnd, request.anchor,
                        request.pageStart, request.pageSize, request.includeAll ? 1 : 0));
                if (o.has("error")) {
                    return new FindController.Page(0, 0, 0, 0, 0,
                            new FindController.Match[0], o.getString("error"));
                }
                trace("findScope", "scope", scope, "query", deviceTest ? request.query : "",
                        "total", o.optInt("total"), "replaceable", o.optInt("replaceable"));
                JSONArray array = o.optJSONArray("matches");
                FindController.Match[] matches = new FindController.Match[
                        array == null ? 0 : array.length()];
                for (int i = 0; i < matches.length; i++) {
                    JSONObject match = array.getJSONObject(i);
                    matches[i] = new FindController.Match(match.getInt("start"),
                            match.getInt("end"), match.optBoolean("protected"));
                }
                return new FindController.Page(o.optInt("total"), o.optInt("replaceable"),
                        o.optInt("protected"), o.optInt("first"),
                        o.optInt("pageStart"), matches, null);
            }
        }, findWorker, new java.util.concurrent.Executor() {
            @Override public void execute(Runnable command) { main.post(command); }
        }, new FindController.Listener() {
            @Override public void onFindChanged() {
                renderFindState();
                if (find != null && !find.isLoading()) showCurrentFindMatch();
            }
        });
    }

    private void openSearch() { openSearch("", false); }

    private void openSearch(String initialQuery, boolean previous) {
        if (handle == 0) return;
        ensureFind();
        if (searchBar.getVisibility() != View.VISIBLE) {
            rememberReadingAnchor();
            searchOriginState = new SearchOrigin(editor.getSelectionStart(), editor.getSelectionEnd(),
                    previewScroll.getScrollY(), readingAnchorStable, readingAnchorFallback,
                    new java.util.LinkedHashMap<>(expandedSegments), inspectorOpen);
            find.open(editor.getText().toString(), editor.getSelectionStart(),
                    editor.getSelectionStart(), editor.getSelectionEnd());
            if (inspectorOpen) closeInspector();
            searchBar.setVisibility(View.VISIBLE);
            replaceVisible = false;
            replaceRow.setVisibility(View.GONE);
            replaceStatus.setVisibility(View.GONE);
            replaceStatus.setText("");
            replaceField.setText("");
            replacePlan = null;
            replacePlanText = "";
            findResultsSignature = "";
            searchField.setText("");
            searchCount.setText("");
            renderFindState();
        }
        if (!initialQuery.isEmpty()) {
            searchField.setText(initialQuery);
            searchField.setSelection(initialQuery.length());
            if (previous) {
                main.post(new Runnable() {
                    @Override public void run() {
                        if (find != null && !find.isLoading()) find.previous(editor.getText().toString());
                    }
                });
            }
        }
        searchField.requestFocus();
    }

    private void updateSearch() {
        if (handle == 0 || searchBar.getVisibility() != View.VISIBLE || find == null) return;
        String query = searchField.getText().toString();
        String text = editor.getText().toString();
        if (!find.query().equals(query)) {
            replacePlan = null;
            replacePlanText = "";
            findResultsSignature = "";
            resetReplaceButton();
            find.queryChanged(text, query);
        } else {
            find.documentChanged(text);
        }
    }

    private void clearReplacePlan() {
        replacePlan = null;
        replacePlanText = "";
        resetReplaceButton();
    }

    private void resetReplaceButton() {
        if (replaceAllButton != null) replaceAllButton.setText("Replace all…");
    }

    private void moveFind(int direction) {
        if (find == null || find.total() == 0) return;
        if (direction < 0) find.previous(editor.getText().toString());
        else find.next(editor.getText().toString());
    }

    private void cycleFindScope() {
        if (find == null) return;
        int next = find.scope() == FindController.SCOPE_TEXT ? FindController.SCOPE_SOURCE
                : find.scope() == FindController.SCOPE_SOURCE ? FindController.SCOPE_SELECTION
                : FindController.SCOPE_TEXT;
        if (next == FindController.SCOPE_SELECTION && !find.selectionAvailable()) {
            next = FindController.SCOPE_TEXT;
        }
        clearReplacePlan();
        find.setScope(editor.getText().toString(), next);
        findResultsSignature = "";
        renderFindState();
    }

    private void showCurrentFindMatch() {
        if (find == null || searchBar.getVisibility() != View.VISIBLE) return;
        FindController.Match match = find.currentMatch();
        if (match == null) {
            findScrollPending = false;
            invalidateRenderedCaret();
            return;
        }
        clearSelectionAnchor();
        editor.setSelection(match.start, match.end);
        if (match.start < match.end) setSelectionAnchor(match.start);
        invalidateRenderedCaret();
        if (find.scope() == FindController.SCOPE_SOURCE && match.protectedRange) {
            findScrollPending = false;
            searchCount.setText(find.status() + " \u00b7 Accept opens source");
            return;
        }
        revealFoldAt(match.start);
        scrollFindMatch(match);
    }

    private void scrollFindMatch(FindController.Match match) {
        if (searchBar.getVisibility() != View.VISIBLE) return;
        final int target = match.start;
        CaretSpot spot = caretSpot(target);
        if (spot.page != null && spot.distance == 0) {
            findScrollPending = false;
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
            findScrollPending = true;
            locatePage(target, "search");
        }
    }

    /** Reveal the innermost collapsed segments containing an accepted match. */
    private void revealFoldAt(int offset) {
        boolean changed = false;
        for (int i = 0; i < documentSegments.length(); i++) {
            JSONObject segment = documentSegments.optJSONObject(i);
            if (segment == null) continue;
            JSONArray span = segment.optJSONArray("span");
            if (span == null || span.length() != 2) continue;
            if (offset < span.optInt(0) || offset > span.optInt(1)) continue;
            String key = segment.optString("startId") + "\u0000" + segment.optString("endId");
            if (expandedSegments.containsKey(key)) continue;
            expandedSegments.put(key, new String[] {segment.optString("startId"),
                    segment.optString("endId")});
            changed = true;
        }
        if (changed) scheduleRender();
    }

    private void acceptFind() {
        if (find == null || find.currentMatch() == null) {
            closeSearch(false);
            return;
        }
        FindController.Match match = find.currentMatch();
        boolean source = find.scope() == FindController.SCOPE_SOURCE && match.protectedRange;
        rememberFindQuery();
        recordAcceptedFind(match.start, match.end, source);
        closeSearch(false);
        if (source) openSourceAt(match.start, match.end);
    }

    private void rememberFindQuery() {
        if (find == null || find.query().isEmpty()) return;
        prefs().edit().putString("findQuery:" + session.documentId(), find.query()).apply();
    }

    private String lastFindQuery() {
        return prefs().getString("findQuery:" + session.documentId(), "");
    }

    private void openSourceAt(int start, int end) {
        openSourceAt(start, end, true);
    }

    private void openSourceAt(int start, int end, boolean wholeDocument) {
        if (inspectorOpen) closeInspector();
        clearSelectionAnchor();
        editor.setSelection(Math.max(0, start), Math.max(0, end));
        inspectorOpen = true;
        wholeDocumentEditing = wholeDocument;
        inspectorSignature = "";
        // The inspector opens with the source field focused, so the IME already
        // takes ~400dp of a tall tablet. Keep the preview's weighted share too
        // and the relationship list is squeezed to ~130dp -- two rows of five,
        // reachable only by scrolling. The preview is the reading surface and
        // the inspector is the editing one; nobody needs both at once. Only a
        // user-intent open yields it: a cold start that restores the inspector
        // still has to lay the preview out to apply the saved reading offset.
        previewScroll.setVisibility(View.GONE);
        updateModifierPanel();
        focusSourceCaret();
        if (sourceField != null) {
            int a = Math.max(0, start - modifierBodyStart);
            int b = Math.max(a, end - modifierBodyStart);
            sourceField.setSelection(Math.min(a, sourceField.length()),
                    Math.min(b, sourceField.length()));
        }
    }

    private void restoreSearchOrigin() {
        SearchOrigin origin = searchOriginState;
        if (origin == null) return;
        editor.setSelection(origin.caretStart, origin.caretEnd);
        if (origin.caretStart < origin.caretEnd) setSelectionAnchor(origin.caretStart);
        else clearSelectionAnchor();
        boolean foldsChanged = !expandedSegments.equals(origin.folds);
        expandedSegments.clear();
        expandedSegments.putAll(origin.folds);
        readingAnchorStable = origin.reading;
        readingAnchorFallback = origin.readingFallback;
        if (foldsChanged) {
            scrollToReadingAnchorAfterChunk = origin.reading != null;
            scheduleRender();
            if (!scrollToReadingAnchor()) previewScroll.scrollTo(0, origin.scrollY);
        } else {
            // Search overlays never reflow the page; restore the exact viewport.
            scrollToReadingAnchorAfterChunk = false;
            previewScroll.scrollTo(0, origin.scrollY);
        }
        if (origin.inspector) {
            inspectorOpen = true;
            wholeDocumentEditing = false;
            inspectorSignature = "";
            updateModifierPanel();
            focusSourceCaret();
        }
    }

    private void closeSearch(boolean cancel) {
        if (cancel) restoreSearchOrigin();
        searchBar.setVisibility(View.GONE);
        findResultsScroll.setVisibility(View.GONE);
        replaceRow.setVisibility(View.GONE);
        replaceStatus.setVisibility(View.GONE);
        replaceVisible = false;
        findScrollPending = false;
        if (find != null) find.cancel();
        searchOriginState = null;
        android.view.inputmethod.InputMethodManager ime =
                (android.view.inputmethod.InputMethodManager) getSystemService(INPUT_METHOD_SERVICE);
        if (ime != null) ime.hideSoftInputFromWindow(searchField.getWindowToken(), 0);
        invalidateRenderedCaret();
        if (!cancel && !inspectorOpen) focusRenderedCaret();
    }

    private void renderFindState() {
        if (find == null || searchBar == null || searchBar.getVisibility() != View.VISIBLE) return;
        String scope = find.scope() == FindController.SCOPE_SOURCE ? "Source"
                : find.scope() == FindController.SCOPE_SELECTION ? "Selection" : "Text";
        findScopeButton.setText(scope);
        findScopeButton.setContentDescription("Search scope: " + scope);
        searchCount.setText(find.status());
        setStatus(statusText);
        findResultsScroll.setVisibility(find.pageMatches().length == 0 ? View.GONE : View.VISIBLE);
        String signature = find.query() + "\u0000" + find.scope() + "\u0000"
                + find.pageStart() + "\u0000" + find.total() + "\u0000" + find.pageMatches().length;
        if (signature.equals(findResultsSignature)) return;
        findResultsSignature = signature;
        findResults.removeAllViews();
        for (int i = 0; i < find.pageMatches().length; i++) {
            final int global = find.pageStart() + i;
            final FindController.Match match = find.pageMatches()[i];
            TextView row = panelLabel("");
            row.setText(findSnippet(match, global));
            row.setTextColor(match.protectedRange ? Color.parseColor("#D6A86A")
                    : Color.parseColor("#C8D4DF"));
            row.setPadding(dp(6), dp(5), dp(6), dp(5));
            row.setContentDescription("Match " + (global + 1) + " of " + find.total());
            row.setOnClickListener(new View.OnClickListener() {
                @Override public void onClick(View v) {
                    if (find != null) find.selectGlobal(global, editor.getText().toString());
                }
            });
            findResults.addView(row);
            if (i + 1 < find.pageMatches().length) {
                View divider = new View(this);
                divider.setBackgroundColor(Color.parseColor("#25313C"));
                findResults.addView(divider, new LinearLayout.LayoutParams(
                        LinearLayout.LayoutParams.MATCH_PARENT, dp(1)));
            }
        }
    }

    /** One 80-grapheme source snippet with the current match marked. */
    private CharSequence findSnippet(FindController.Match match, int global) {
        String text = editor.getText().toString();
        int start = Math.max(0, Math.min(match.start, text.length()));
        int end = Math.max(start, Math.min(match.end, text.length()));
        int before = start, after = end;
        for (int i = 0; i < 40 && before > 0; i++) {
            before = DocBridge.graphemeBoundary(text, before, -1);
        }
        for (int i = 0; i < 40 && after < text.length(); i++) {
            after = DocBridge.graphemeBoundary(text, after, 2);
        }
        SpannableStringBuilder snippet = new SpannableStringBuilder();
        if (before > 0) snippet.append("\u2026");
        int contentStart = snippet.length();
        snippet.append(text, before, after);
        if (after < text.length()) snippet.append("\u2026");
        int markStart = contentStart + (start - before);
        int markEnd = contentStart + (end - before);
        if (markEnd > markStart) {
            snippet.setSpan(new BackgroundColorSpan(0x6689B06A), markStart, markEnd,
                    Spannable.SPAN_EXCLUSIVE_EXCLUSIVE);
        }
        snippet.append("  ");
        snippet.append(match.protectedRange ? "\u00b7 structural" : "\u00b7 @" + (start + 1));
        return snippet;
    }

    private void toggleReplaceRow() {
        replaceVisible = !replaceVisible;
        replaceRow.setVisibility(replaceVisible ? View.VISIBLE : View.GONE);
        replaceStatus.setVisibility(replaceVisible ? View.VISIBLE : View.GONE);
        if (replaceVisible) {
            replaceField.requestFocus();
            replaceField.setSelection(replaceField.length());
        } else {
            clearReplacePlan();
            replaceField.clearFocus();
            searchField.requestFocus();
        }
    }

    private void replaceCurrent() {
        if (find == null || find.currentMatch() == null || handle == 0) return;
        FindController.Match match = find.currentMatch();
        if (match.protectedRange) {
            replaceStatus.setText("Structural matches are not replaced");
            return;
        }
        String expected = editor.getText().toString();
        JSONArray ops = new JSONArray();
        JSONObject op = new JSONObject();
        try {
            op.put("start", match.start);
            op.put("end", match.end);
            op.put("text", replaceField.getText().toString());
            ops.put(op);
            String state = DocBridge.replaceMany(handle, expected, ops.toString());
            JSONObject result = new JSONObject(state);
            if (result.has("error")) {
                replaceStatus.setText(result.getString("error"));
                clearReplacePlan();
                return;
            }
            replaceUndoStart = editor.getSelectionStart();
            replaceUndoEnd = editor.getSelectionEnd();
            applyState(state, true);
            find.searchFrom(editor.getText().toString(), editor.getSelectionStart());
            replaceStatus.setText("Replaced 1 occurrence");
            clearReplacePlan();
        } catch (Exception e) {
            replaceStatus.setText("Replacement failed");
            Log.w("emthin-pad", "literal replacement failed", e);
        }
    }

    private void previewOrApplyReplaceAll() {
        if (find == null || handle == 0) return;
        String current = editor.getText().toString();
        if (replacePlan != null && replacePlanText.equals(current)
                && find.query().equals(searchField.getText().toString())) {
            applyReplacePlan(replacePlan, current);
            return;
        }
        final String expected = current;
        replaceStatus.setText("Preparing replacement\u2026");
        resetReplaceButton();
        find.requestPlan(expected, new FindController.PlanListener() {
            @Override public void onPlan(FindController.Page page, String error) {
                if (searchBar.getVisibility() != View.VISIBLE || find == null) return;
                if (error != null || page.error != null) {
                    replaceStatus.setText(error != null ? error : page.error);
                    return;
                }
                if (!expected.equals(editor.getText().toString())) {
                    replaceStatus.setText("The document changed; refresh the preview");
                    return;
                }
                if (page.replaceable > 10_000) {
                    replaceStatus.setText("Narrow the selection to replace this many occurrences");
                    return;
                }
                if (page.replaceable == 0) {
                    replaceStatus.setText(page.protectedCount > 0
                            ? "Structural matches are not replaced" : "No replaceable matches");
                    return;
                }
                replacePlan = page;
                replacePlanText = expected;
                replaceAllButton.setText("Replace " + page.replaceable + " occurrences");
                String replacement = replaceField.getText().toString();
                StringBuilder preview = new StringBuilder();
                preview.append(page.replaceable).append(page.replaceable == 1
                        ? " replaceable match" : " replaceable matches");
                if (page.protectedCount > 0) {
                    preview.append(" \u00b7 ").append(page.protectedCount)
                            .append(" protected \u00b7 Structural matches are not replaced");
                }
                int shown = 0;
                for (int i = 0; i < page.matches.length && shown < 20; i++) {
                    FindController.Match match = page.matches[i];
                    if (match.protectedRange) continue;
                    String before = editor.getText().subSequence(match.start, match.end).toString();
                    if (before.length() > 40) before = before.substring(0, 40) + "\u2026";
                    String after = replacement.length() > 40
                            ? replacement.substring(0, 40) + "\u2026" : replacement;
                    preview.append("\n").append(before).append(" \u2192 ")
                            .append(after.isEmpty() ? "(delete)" : after);
                    shown++;
                }
                replaceStatus.setText(preview.toString());
            }
        });
    }

    private void applyReplacePlan(FindController.Page page, String expected) {
        JSONArray ops = new JSONArray();
        String replacement = replaceField.getText().toString();
        for (FindController.Match match : page.matches) {
            if (match.protectedRange) continue;
            JSONObject op = new JSONObject();
            try {
                op.put("start", match.start);
                op.put("end", match.end);
                op.put("text", replacement);
                ops.put(op);
            } catch (Exception e) {
                replaceStatus.setText("Replacement plan is invalid");
                return;
            }
        }
        try {
            String state = DocBridge.replaceMany(handle, expected, ops.toString());
            JSONObject result = new JSONObject(state);
            if (result.has("error")) {
                replaceStatus.setText(result.getString("error"));
                clearReplacePlan();
                return;
            }
            replaceUndoStart = editor.getSelectionStart();
            replaceUndoEnd = editor.getSelectionEnd();
            applyState(state, true);
            find.searchFrom(editor.getText().toString(), editor.getSelectionStart());
            replaceStatus.setText("Replaced " + page.replaceable + " occurrences");
            clearReplacePlan();
        } catch (Exception e) {
            replaceStatus.setText("Replacement failed");
            Log.w("emthin-pad", "replace-all failed", e);
        }
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

    /** Capture the reader's actual place, including stable source anchors. */
    private NavigationController.Entry currentLocation(String kind, String label) {
        rememberReadingAnchor();
        int a = editor == null ? 0 : editor.getSelectionStart();
        int b = editor == null ? 0 : editor.getSelectionEnd();
        int start = Math.min(a, b), end = Math.max(a, b);
        StableAnchor selection = selectionAnchor >= 0 ? captureStableAnchor(selectionAnchor, -1) : null;
        return new NavigationController.Entry(kind, label, start, end,
                selection == null ? "" : selection.encoded,
                selection == null ? -1 : selection.side,
                selection == null ? start : selection.fallback,
                readingAnchorStable == null ? "" : readingAnchorStable.encoded,
                readingAnchorStable == null ? -1 : readingAnchorStable.side,
                readingAnchorFallback, expandedSegments, inspectorOpen, wholeDocumentEditing);
    }

    private NavigationController.Entry destination(String kind, String label,
                                                  int start, int end, boolean inspector) {
        int a = Math.max(0, Math.min(editor.length(), start));
        int b = Math.max(0, Math.min(editor.length(), end));
        StableAnchor anchor = captureStableAnchor(a, -1);
        return new NavigationController.Entry(kind, label, Math.min(a, b), Math.max(a, b),
                anchor == null ? "" : anchor.encoded, anchor == null ? -1 : anchor.side,
                anchor == null ? a : anchor.fallback,
                anchor == null ? "" : anchor.encoded, anchor == null ? -1 : anchor.side,
                anchor == null ? a : anchor.fallback, expandedSegments, inspector, inspector);
    }

    private void resetNavigation() {
        navigation.reset(currentLocation("current", "Current"));
    }

    /** Explicit destinations only: accepted search, sections, definitions/uses,
     * references and diagnostics. Ordinary caret/scroll/fold changes never
     * enter here. */
    private void navigateTo(String kind, String label, int start, int end, boolean inspector) {
        if (handle == 0) return;
        if (navigation.current() == null) resetNavigation();
        else navigation.replaceCurrent(currentLocation("current", "Current"));
        NavigationController.Entry destination = destination(kind, label, start, end, inspector);
        if (!navigation.push(destination)) return;
        NavigationController.Entry origin = navigation.entries().get(navigation.index() - 1);
        trace("navigate", "kind", kind, "start", start, "history", navigation.size(),
                "index", navigation.index(), "inspector", inspector,
                "originInspector", origin.inspector,
                "originWholeDocument", origin.wholeDocument);
        restoreLocation(destination);
    }

    private void recordAcceptedFind(int start, int end, boolean source) {
        if (handle == 0) return;
        NavigationController.Entry origin = null;
        SearchOrigin saved = searchOriginState;
        if (saved != null) {
            origin = new NavigationController.Entry("current", "Current",
                    Math.min(saved.caretStart, saved.caretEnd),
                    Math.max(saved.caretStart, saved.caretEnd), "", -1, saved.caretStart,
                    saved.reading == null ? "" : saved.reading.encoded,
                    saved.reading == null ? -1 : saved.reading.side, saved.readingFallback,
                    saved.folds, saved.inspector, false);
        }
        if (navigation.current() == null) {
            navigation.reset(origin == null ? currentLocation("current", "Current") : origin);
        } else {
            navigation.replaceCurrent(origin == null ? currentLocation("current", "Current") : origin);
        }
        NavigationController.Entry entry = destination("search", "Accepted find", start, end, source);
        navigation.push(entry);
        trace("navigate", "kind", "search", "start", start, "history", navigation.size(),
                "index", navigation.index());
    }

    private void navigateBack() {
        NavigationController.Entry entry = navigation.back();
        if (entry != null) {
            trace("navigate", "kind", entry.kind, "start", entry.start,
                    "history", navigation.size(), "index", navigation.index());
            restoreLocation(entry);
        }
    }

    private void navigateForward() {
        NavigationController.Entry entry = navigation.forward();
        if (entry != null) {
            trace("navigate", "kind", entry.kind, "start", entry.start,
                    "history", navigation.size(), "index", navigation.index());
            restoreLocation(entry);
        }
    }

    /** Restore source selection, folds and reading position. A destination whose
     * rendered body is unavailable falls back to the source inspector. */
    private void restoreLocation(NavigationController.Entry entry) {
        if (handle == 0 || entry == null) return;
        if (searchBar != null && searchBar.getVisibility() == View.VISIBLE) closeSearch(false);
        boolean foldsChanged = !expandedSegments.equals(entry.folds);
        expandedSegments.clear();
        expandedSegments.putAll(entry.folds);
        int start = Math.max(0, Math.min(editor.length(), entry.start));
        int end = Math.max(0, Math.min(editor.length(), entry.end));
        if (entry.inspector) {
            openSourceAt(start, Math.max(start, end), entry.wholeDocument);
            if (foldsChanged) scheduleRender();
            return;
        }
        if (inspectorOpen) closeInspector();
        clearSelectionAnchor();
        editor.setSelection(start, end);
        if (start < end) {
            if (!entry.selectionAnchor.isEmpty()) {
                selectionAnchor = start;
                selectionAnchorStable = new StableAnchor(session.documentId(),
                        entry.selectionAnchor, entry.selectionSide, entry.selectionFallback);
                showSelectionBar();
            } else {
                setSelectionAnchor(start);
            }
        }
        readingAnchorStable = entry.readingAnchor.isEmpty() ? null
                : new StableAnchor(session.documentId(), entry.readingAnchor,
                        entry.readingSide, entry.readingFallback);
        readingAnchorFallback = entry.readingFallback;
        if (foldsChanged) scheduleRender();
        if (!scrollToReadingAnchor() && !foldsChanged) {
            if (pagesStale || renderedPages.isEmpty()) {
                openSourceAt(start, Math.max(start, end));
                return;
            }
            locatePage(start, "navigation");
        }
        invalidateRenderedCaret();
    }

    private void refreshSemantics(String text) {
        try {
            documentSemantics = new JSONObject(DocBridge.semanticIndex(text));
            JSONArray defs = documentSemantics.optJSONArray("defs");
            JSONArray sections = documentSemantics.optJSONArray("sections");
            trace("semantics", "defs", defs == null ? 0 : defs.length(),
                    "sections", sections == null ? 0 : sections.length(), "textLen", text.length());
        } catch (Exception e) {
            documentSemantics = new JSONObject();
            Log.w("emthin-pad", "could not read semantic index", e);
        }
    }

    private JSONObject definitionAt(int caret) {
        JSONArray defs = documentSemantics.optJSONArray("defs");
        if (defs == null) return null;
        for (int i = 0; i < defs.length(); i++) {
            JSONObject def = defs.optJSONObject(i);
            JSONArray span = def == null ? null : def.optJSONArray("span");
            if (span != null && caret >= span.optInt(0) && caret <= span.optInt(1)) return def;
            JSONArray uses = def == null ? null : def.optJSONArray("uses");
            if (uses == null) continue;
            for (int j = 0; j < uses.length(); j++) {
                JSONObject use = uses.optJSONObject(j);
                if (use != null && caret >= use.optInt("start") && caret <= use.optInt("end")) return def;
            }
        }
        return null;
    }

    /**
     * K5 read-only history preview. Retains a snapshot of the current note,
     * lists the retained frontiers, and shows a preview on tap. Nothing here
     * writes to the live document, its undo stack or its save baselines.
     */
    private void showHistory() {
        if (handle == 0) { fileMessage("Open a note first"); return; }
        try {
            JSONObject remembered = new JSONObject(DocBridge.historyRemember(handle, "pad"));
            if (remembered.has("error")) { fileMessage(remembered.getString("error")); return; }
            String reply = DocBridge.historyFrontiers(handle);
            JSONArray tokens = new JSONObject(reply).getJSONArray("frontiers");
            if (tokens.length() == 0) { fileMessage("No retained snapshots yet"); return; }
            final PadSession.Owner owner = session.capture(PadSession.HISTORY, "");
            String[] labels = new String[tokens.length()];
            for (int i = 0; i < tokens.length(); i++) {
                labels[i] = i == 0 ? "Latest snapshot" : "Snapshot " + (tokens.length() - i);
            }
            new AlertDialog.Builder(this)
                    .setTitle("Document history")
                    .setItems(labels, (dialog, which) -> {
                        if (!session.sameDocument(owner)) return;
                        try {
                            String preview = DocBridge.historyPreview(handle,
                                    tokens.getString(tokens.length() - 1 - which));
                            JSONObject p = new JSONObject(preview);
                            if (p.has("error")) { fileMessage("Preview: " + p.getString("error")); return; }
                            String text = p.getString("text");
                            String body = text.isEmpty() ? "This snapshot is empty." : text;
                            if (p.optBoolean("truncated")) body += "\n…";
                            new AlertDialog.Builder(MainActivity.this)
                                    .setTitle(labels[which] + " · read-only")
                                    .setMessage(body)
                                    .setPositiveButton("Close", null)
                                    .show();
                        } catch (Exception e) { fileMessage("Preview failed: " + e.getMessage()); }
                    })
                    .setPositiveButton("Close", null)
                    .show();
        } catch (Exception e) {
            fileMessage("History failed: " + e.getMessage());
        }
    }

    /** The Sessions surface: public references and their available actions. */
    // ----- gateway pairing (paste link) -----------------------------------

    /**
     * The Paste-link screen. Pairing itself runs off the UI thread; the status
     * line is the state machine (pending, approved, denied, expired, forbidden,
     * mismatched, unavailable) and only an approved answer adds a row.
     */
    private void showPairing() {
        LinearLayout rows = new LinearLayout(this);
        rows.setOrientation(LinearLayout.VERTICAL);
        rows.setPadding(dp(12), dp(8), dp(12), dp(12));
        ScrollView scroll = new ScrollView(this);
        scroll.addView(rows);
        pairingRows = rows;
        AlertDialog dialog = new AlertDialog.Builder(this, android.R.style.Theme_Material_Dialog_Alert)
                .setTitle("Pair a gateway")
                .setView(scroll)
                .setPositiveButton("Paste link", new DialogInterface.OnClickListener() {
                    @Override public void onClick(DialogInterface d, int which) { showPasteLink(); }
                })
                .setNeutralButton("Scan QR", new DialogInterface.OnClickListener() {
                    @Override public void onClick(DialogInterface d, int which) { scanPairingQr(); }
                })
                .setNegativeButton("Close", null)
                .create();
        dialog.setOnDismissListener(new DialogInterface.OnDismissListener() {
            @Override public void onDismiss(DialogInterface d) {
                pairingRows = null;
                if (pairingDialog == dialog) pairingDialog = null;
            }
        });
        pairingDialog = dialog;
        renderPairingRows();
        dialog.show();
        trace("gatewayPair", "phase", "screen");
    }

    /**
     * The camera half of Pair: a scan only ever produces the same link text a
     * paste does, so it lands in {@link #pairFromText} and nothing else.
     */
    private void scanPairingQr() {
        // The scanner is a full activity, so the dialog that launched it goes
        // away for the duration; the result handler brings the screen back.
        if (pairingDialog != null) {
            pairingDialog.dismiss();
            pairingDialog = null;
        }
        try {
            startActivityForResult(new Intent(this, QrScanActivity.class), REQUEST_QR);
            trace("gatewayPair", "phase", "scan");
        } catch (RuntimeException e) {
            setPairingStatus("Scanner unavailable \u2014 " + e.getMessage());
            showPairing();
        }
    }

    /** The scanner's answer: the same link text, through the same parse. */
    private void completeQrScan(int resultCode, Intent data) {
        String text = resultCode == RESULT_OK && data != null
                ? data.getStringExtra(QrScanActivity.EXTRA_TEXT) : null;
        if (text == null || text.trim().isEmpty()) {
            String reason = data == null ? "cancelled"
                    : data.getStringExtra(QrScanActivity.EXTRA_REASON);
            trace("gatewayPair", "phase", "scan-failed", "reason", String.valueOf(reason));
            setPairingStatus("Scanner unavailable \u2014 " + reason);
            showPairing();
            return;
        }
        showPairing();
        pairFromText(text, null);
    }

    /**
     * One path for a pasted or scanned link: the same parse, the same status on
     * a rejection and the same request.
     *
     * @param error field to put a rejection in, or null to report it on the
     *     pairing screen
     * @return the parsed link, or null when the text is not one
     */
    private PairingLink pairFromText(String text, TextView error) {
        PairingLink link;
        try {
            link = PairingLink.parse(text);
        } catch (IllegalArgumentException e) {
            String reason = String.valueOf(e.getMessage());
            if (error != null) {
                error.setText("Link not accepted: " + reason);
            } else {
                setPairingStatus("Link not accepted \u2014 " + reason);
            }
            trace("gatewayPair", "phase", "rejected", "reason", reason);
            return null;
        }
        startPairing(link);
        return link;
    }

    private void renderPairingRows() {
        LinearLayout rows = pairingRows;
        if (rows == null) return;
        rows.removeAllViews();
        rows.addView(panelLabel("Pairing is approved on the host. Only an approved answer for the Core ID in the link is stored."));
        TextView status = panelLabel(pairingStatusText);
        status.setContentDescription(pairingStatusText);
        rows.addView(status);
        List<GatewayEndpoints.Endpoint> known = gatewayEndpoints == null
                ? new ArrayList<GatewayEndpoints.Endpoint>() : gatewayEndpoints.list();
        if (known.isEmpty()) {
            rows.addView(panelLabel("No gateways paired yet."));
            return;
        }
        for (final GatewayEndpoints.Endpoint endpoint : known) {
            LinearLayout card = new LinearLayout(this);
            card.setOrientation(LinearLayout.VERTICAL);
            card.setPadding(0, dp(8), 0, dp(8));
            TextView heading = panelLabel(endpoint.name + " \u00b7 " + endpoint.host);
            heading.setContentDescription(endpoint.name + ", " + endpoint.host);
            card.addView(heading);
            LinearLayout actions = new LinearLayout(this);
            actions.setOrientation(LinearLayout.HORIZONTAL);
            actions.addView(shortcut("Forget", new Runnable() {
                @Override public void run() { forgetEndpoint(endpoint); }
            }));
            card.addView(actions);
            rows.addView(card);
        }
    }

    /** Forget removes the row here; it never revokes anything on the host. */
    private void forgetEndpoint(GatewayEndpoints.Endpoint endpoint) {
        if (gatewayEndpoints == null) return;
        gatewayEndpoints.remove(endpoint.host);
        setPairingStatus("Forgot " + endpoint.name + " \u2014 revoke the grant on the host to complete this");
        trace("gatewayPair", "phase", "forget", "host", endpoint.host);
    }

    private void showPasteLink() {
        LinearLayout box = new LinearLayout(this);
        box.setOrientation(LinearLayout.VERTICAL);
        box.setPadding(dp(16), dp(8), dp(16), dp(4));
        final EditText field = new EditText(this);
        field.setHint("https://host.tailnet.ts.net/pair?v=1&core=\u2026");
        field.setSingleLine(false);
        field.setMaxLines(3);
        field.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_URI);
        box.addView(field);
        final TextView error = panelLabel("");
        box.addView(error);
        final AlertDialog dialog = new AlertDialog.Builder(this, android.R.style.Theme_Material_Dialog_Alert)
                .setTitle("Paste pairing link")
                .setView(box)
                .setPositiveButton("Pair", null)
                .setNeutralButton("Paste", null)
                .setNegativeButton("Cancel", null)
                .create();
        dialog.setOnShowListener(new DialogInterface.OnShowListener() {
            @Override public void onShow(DialogInterface d) {
                dialog.getButton(AlertDialog.BUTTON_NEUTRAL).setOnClickListener(new View.OnClickListener() {
                    @Override public void onClick(View v) {
                        String text = clipText();
                        if (text != null && !text.trim().isEmpty()) field.setText(text.trim());
                        field.setSelection(field.getText().length());
                    }
                });
                // The positive button is overridden so a rejected link keeps
                // the field open with the reason instead of closing.
                dialog.getButton(AlertDialog.BUTTON_POSITIVE).setOnClickListener(new View.OnClickListener() {
                    @Override public void onClick(View v) {
                        if (pairFromText(field.getText().toString(), error) == null) return;
                        dialog.dismiss();
                        // The request's state is reported on the pairing screen,
                        // and showing the paste field dismissed it; bring it
                        // back rather than pairing with nothing on screen.
                        showPairing();
                    }
                });
            }
        });
        dialog.show();
    }

    private String clipText() {
        android.content.ClipboardManager clipboard =
                (android.content.ClipboardManager) getSystemService(CLIPBOARD_SERVICE);
        if (clipboard == null || !clipboard.hasPrimaryClip() || clipboard.getPrimaryClip() == null
                || clipboard.getPrimaryClip().getItemCount() == 0) return null;
        CharSequence value = clipboard.getPrimaryClip().getItemAt(0).coerceToText(this);
        return value == null ? null : value.toString();
    }

    private void startPairing(final PairingLink link) {
        if (pairingInFlight) {
            setPairingStatus("A pairing request is already waiting for approval");
            return;
        }
        pairingInFlight = true;
        setPairingStatus("Pending on " + link.displayName() + " \u2014 approve this device on the host");
        trace("gatewayPair", "phase", "request", "host", link.host, "core", link.coreId);
        new Thread(new Runnable() {
            @Override public void run() {
                try {
                    HttpsGatewayHttp http;
                    if (fixturePairBase == null || fixturePairBase.trim().isEmpty()) {
                        http = new HttpsGatewayHttp();
                    } else {
                        http = new HttpsGatewayHttp(fixturePairBase.trim(),
                                PemTrust.load(fixtureGatewayCa == null
                                        ? "" : fixtureGatewayCa.trim()));
                    }
                    PairingProtocol.Result result = PairingProtocol.start(http, link);
                    while (!result.state.terminal()) {
                        final int expires = result.expiresInS;
                        main.post(new Runnable() {
                            @Override public void run() {
                                setPairingStatus("Pending on " + link.displayName()
                                        + (expires > 0
                                        ? " \u2014 approve this device on the host (" + expires + "s)"
                                        : " \u2014 approve this device on the host"));
                            }
                        });
                        Thread.sleep(PairingProtocol.POLL_MS);
                        result = PairingProtocol.poll(http, link, result.requestId);
                    }
                    final PairingProtocol.Result done = result;
                    main.post(new Runnable() {
                        @Override public void run() { completePairing(link, done); }
                    });
                } catch (Exception e) {
                    main.post(new Runnable() {
                        @Override public void run() {
                            pairingInFlight = false;
                            setPairingStatus("Unavailable \u2014 " + link.displayName() + " did not answer");
                            trace("gatewayPair", "phase", "unavailable", "host", link.host);
                        }
                    });
                }
            }
        }, "gateway-pairing").start();
    }

    private void completePairing(PairingLink link, PairingProtocol.Result result) {
        pairingInFlight = false;
        switch (result.state) {
            case APPROVED:
                if (gatewayEndpoints != null
                        && gatewayEndpoints.add(link.host, link.coreId, link.displayName())) {
                    setPairingStatus("Approved \u2014 " + link.displayName() + " is paired");
                } else {
                    setPairingStatus("Approved, but the link could not be stored");
                }
                break;
            case DENIED:
                setPairingStatus("Denied \u2014 the host refused this device");
                break;
            case EXPIRED:
                setPairingStatus("Expired \u2014 the request timed out; start again");
                break;
            case FORBIDDEN:
                setPairingStatus("Forbidden \u2014 this network is not allowed to pair");
                break;
            case CORE_MISMATCH:
                setPairingStatus("Mismatched \u2014 the host approved a different Core");
                break;
            case UNAVAILABLE:
                setPairingStatus("Unavailable \u2014 the host is offline or rate limiting");
                break;
            default:
                setPairingStatus("Invalid \u2014 the host's reply was not understood");
                break;
        }
        trace("gatewayPair", "phase", result.describe(), "host", link.host,
                "state", result.state.name());
    }

    private void setPairingStatus(String text) {
        pairingStatusText = text;
        renderPairingRows();
    }

    private void showSessions() {
        ScrollView scroll = new ScrollView(this);
        LinearLayout rows = new LinearLayout(this);
        rows.setOrientation(LinearLayout.VERTICAL);
        rows.setPadding(dp(12), dp(8), dp(12), dp(12));
        scroll.addView(rows);
        sessionsRows = rows;
        AlertDialog dialog = new AlertDialog.Builder(this, android.R.style.Theme_Material_Dialog_Alert)
                .setTitle("Sessions")
                .setView(scroll)
                .setNegativeButton("Close", null)
                .setNeutralButton("Refresh", new DialogInterface.OnClickListener() {
                    @Override public void onClick(DialogInterface d, int which) { refreshSessionCatalog(); }
                })
                .create();
        dialog.setOnDismissListener(new DialogInterface.OnDismissListener() {
            @Override public void onDismiss(DialogInterface d) { sessionsRows = null; }
        });
        renderSessionRows();
        dialog.show();
        refreshSessionCatalog();
    }

    /**
     * The open picker's contents: one row per paired host (its last answer or
     * the reason it had none, with Reconnect), then every host's sessions.
     */
    private void renderSessionRows() {
        LinearLayout rows = sessionsRows;
        if (rows == null) return;
        rows.removeAllViews();
        rows.addView(panelLabel("Sessions are public references; no socket path or credential is shown."));
        if (sessionRefreshInFlight) rows.addView(panelLabel("Refreshing \u2026"));
        List<SessionCatalog.Host> hosts = sessionCatalog.hosts();
        if (hosts.isEmpty()) {
            rows.addView(panelLabel(gatewayEndpoints == null || gatewayEndpoints.list().isEmpty()
                    ? "No gateways paired yet \u2014 pair one to list its sessions."
                    : "No host has answered yet."));
        }
        for (final SessionCatalog.Host host : hosts) {
            LinearLayout card = new LinearLayout(this);
            card.setOrientation(LinearLayout.VERTICAL);
            card.setPadding(0, dp(8), 0, dp(8));
            String state = host.reachable() ? "answered" : host.error;
            TextView heading = panelLabel(host.label + " \u00b7 " + host.host + " \u00b7 " + state);
            heading.setContentDescription(host.label + ", " + host.host + ", " + state);
            card.addView(heading);
            LinearLayout actions = new LinearLayout(this);
            actions.setOrientation(LinearLayout.HORIZONTAL);
            TextView reconnect = shortcut("Reconnect", new Runnable() {
                @Override public void run() { reconnectHost(host); }
            });
            reconnect.setEnabled(!sessionRefreshInFlight);
            actions.addView(reconnect);
            card.addView(actions);
            rows.addView(card);
        }
        for (final SessionCatalog.Entry entry : sessionCatalog.entries()) {
            LinearLayout card = new LinearLayout(this);
            card.setOrientation(LinearLayout.VERTICAL);
            card.setPadding(0, dp(8), 0, dp(8));
            TextView heading = panelLabel(entry.caption + " · " + entry.stateLabel()
                    + " · " + entry.hostLabel);
            heading.setContentDescription(entry.caption + ", " + entry.stateLabel()
                    + ", on " + entry.hostLabel);
            card.addView(heading);
            LinearLayout actions = new LinearLayout(this);
            actions.setOrientation(LinearLayout.HORIZONTAL);
            TextView pin = shortcut(SessionCatalog.actionLabel("PIN", entry), new Runnable() {
                @Override public void run() { pinSession(entry); }
            });
            pin.setEnabled(entry.canPin());
            actions.addView(pin);
            actions.addView(shortcut(SessionCatalog.actionLabel("REBIND", entry), new Runnable() {
                @Override public void run() { showRebindDialog(entry); }
            }));
            actions.addView(shortcut(SessionCatalog.actionLabel("GO_TO_FIGURE", entry), new Runnable() {
                @Override public void run() { goToSession(entry); }
            }));
            TextView detach = shortcut(SessionCatalog.actionLabel("DETACH", entry), new Runnable() {
                @Override public void run() {
                    sessionCatalog = sessionCatalog.detach(entry.reference);
                    renderSessionRows();
                    fileMessage("Detached " + entry.caption + "; the session stays on its host");
                }
            });
            detach.setEnabled(entry.canDetach());
            actions.addView(detach);
            card.addView(actions);
            rows.addView(card);
        }
    }

    /**
     * Retry one host. A failed gateway is retried from its row instead of the
     * whole picker being an error, and every host's rows are rebuilt together
     * because a refresh is one merge, not per-host patching.
     */
    /**
     * Reconnect one host. Only its rows and its outcome are replaced — a host
     * that did not answer must not cost the others their catalogue, which is
     * what a fleet-wide refresh would do when it is the dead one being retried.
     */
    private void reconnectHost(final SessionCatalog.Host host) {
        GatewayEndpoints.Endpoint endpoint = gatewayEndpoints == null
                ? null : gatewayEndpoints.find(host.host);
        if (endpoint == null) {
            trace("sessions", "phase", "reconnect", "host", host.host, "failed", "unpaired");
            return;
        }
        if (sessionRefreshInFlight) return;
        trace("sessions", "phase", "reconnect", "host", host.host);
        final GatewayEndpoints.Endpoint target = endpoint;
        final Set<String> pinned = pinnedReferences();
        sessionRefreshInFlight = true;
        if (sessionsRows != null) renderSessionRows();
        Thread worker = new Thread(new Runnable() {
            @Override public void run() {
                final GatewayCatalog.HostResult result =
                        GatewayCatalog.one(target, new GatewayCatalog.Fetcher() {
                            @Override public List<SessionCatalog.Entry> fetch(
                                    GatewayEndpoints.Endpoint endpoint) throws java.io.IOException {
                                return fetchHostCatalogue(endpoint);
                            }
                        });
                main.post(new Runnable() {
                    @Override public void run() {
                        sessionRefreshInFlight = false;
                        sessionCatalog = sessionCatalog.withHost(result.host,
                                result.rows == null ? new ArrayList<SessionCatalog.Entry>()
                                        : result.rows, pinned);
                        if (sessionsRows != null) renderSessionRows();
                        trace("sessions", "phase", "host", result.host.host,
                                result.rows == null ? "failed" : "rows",
                                result.rows == null ? String.valueOf(result.host.error)
                                        : String.valueOf(result.rows.size()));
                    }
                });
            }
        }, "emthin-sessions-host");
        worker.setDaemon(true);
        worker.start();
    }

    /**
     * Rebuild the picker from every paired host. Runs on a worker thread: each
     * host is one connect + hello + `sessions.list`, and a dead one costs its
     * own timeout rather than the picker's.
     */
    private void refreshSessionCatalog() {
        if (gatewayEndpoints == null || sessionRefreshInFlight) return;
        final List<GatewayEndpoints.Endpoint> endpoints = gatewayEndpoints.list();
        final Set<String> pinned = pinnedReferences();
        sessionRefreshInFlight = true;
        if (sessionsRows != null) renderSessionRows();
        Thread worker = new Thread(new Runnable() {
            @Override public void run() {
                final SessionCatalog fetched = GatewayCatalog.build(endpoints,
                        new GatewayCatalog.Fetcher() {
                            @Override public List<SessionCatalog.Entry> fetch(
                                    GatewayEndpoints.Endpoint endpoint) throws java.io.IOException {
                                try {
                                    List<SessionCatalog.Entry> rows = fetchHostCatalogue(endpoint);
                                    trace("sessions", "phase", "host", endpoint.host,
                                            "rows", String.valueOf(rows.size()));
                                    return rows;
                                } catch (java.io.IOException e) {
                                    trace("sessions", "phase", "host", endpoint.host,
                                            "failed", String.valueOf(e.getMessage()));
                                    throw e;
                                }
                            }
                        }, pinned);
                main.post(new Runnable() {
                    @Override public void run() {
                        sessionRefreshInFlight = false;
                        sessionCatalog = fetched;
                        if (sessionsRows != null) renderSessionRows();
                        trace("sessions", "phase", "catalogue",
                                "hosts", String.valueOf(fetched.hosts().size()),
                                "rows", String.valueOf(fetched.entries().size()));
                    }
                });
            }
        }, "emthin-sessions");
        worker.setDaemon(true);
        worker.start();
    }

    /**
     * One registry row's catalogue, over the same transport its lease uses.
     * `emthin.fixtureConnect` remaps the origin exactly as it does for a lease,
     * so the device lane reaches the scripted Core through a real pair row.
     */
    private List<SessionCatalog.Entry> fetchHostCatalogue(GatewayEndpoints.Endpoint endpoint)
            throws java.io.IOException {
        GatewayTarget target = GatewayTarget.forHost(endpoint.host, endpoint.coreId)
                .remap(fixtureConnect);
        GatewayTransport.Connector connector;
        if (target.secure) {
            javax.net.ssl.SSLContext context = target.fixtureCertificate
                    ? PemTrust.load(fixtureGatewayCa == null ? "" : fixtureGatewayCa.trim())
                    : PemTrust.system();
            connector = WssTransport.connector(target.host, target.port, context, 5000, 5000);
        } else {
            connector = GatewayLeaseClient.tcpConnector(target.host, target.port);
        }
        // The row's label and its sessions' labels are the same string, so a
        // picker row and a session under it never disagree about the host.
        String label = endpoint.name == null || endpoint.name.isEmpty()
                ? endpoint.host : endpoint.name;
        return GatewaySessions.list(connector, endpoint.coreId, endpoint.host, label);
    }

    /** The document's public references; a row they name keeps its Pin row marked. */
    private Set<String> pinnedReferences() {
        Set<String> references = new LinkedHashSet<>();
        java.util.regex.Matcher matcher = java.util.regex.Pattern
                .compile("hc://[^\\s]+|herdr-local://[^\\s]+")
                .matcher(editor.getText());
        while (matcher.find()) {
            String token = matcher.group();
            if (GatewayTarget.Reference.parse(token) != null) references.add(token);
        }
        return references;
    }

    private void pinSession(SessionCatalog.Entry entry) {
        if (handle == 0) return;
        if (goToSession(entry)) return;
        if (editor.getText().toString().contains(entry.reference)) {
            fileMessage(entry.caption + " is already pinned outside the loaded pages");
            return;
        }
        String json = DocBridge.appendTerminalFigure(handle, editor.getText().toString(),
                entry.reference, entry.caption);
        try {
            JSONObject result = new JSONObject(json);
            if (result.has("error")) {
                fileMessage("Pin refused: " + result.optString("error"));
                return;
            }
            applyState(json, true);
            sessionCatalog = sessionCatalog.pin(entry.reference);
            setStatus("Pinned " + entry.caption + " to the document");
        } catch (Exception e) {
            fileMessage("Pin failed: " + e.getMessage());
        }
    }

    /** Pick a document terminal figure and replace only its reference. */
    private void showRebindDialog(final SessionCatalog.Entry entry) {
        if (handle == 0) return;
        JSONArray figures;
        try {
            figures = new JSONArray(DocBridge.terminalFigures(editor.getText().toString()));
        } catch (Exception e) {
            fileMessage("Could not read the document figures");
            return;
        }
        if (figures.length() == 0) {
            fileMessage("This document has no terminal figure to rebind");
            return;
        }
        ScrollView scroll = new ScrollView(this);
        LinearLayout rows = new LinearLayout(this);
        rows.setOrientation(LinearLayout.VERTICAL);
        rows.setPadding(dp(12), dp(8), dp(12), dp(12));
        rows.addView(panelLabel("Rebind " + entry.caption + " to a figure"));
        scroll.addView(rows);
        final AlertDialog[] holder = new AlertDialog[1];
        for (int i = 0; i < figures.length(); i++) {
            final JSONObject figure = figures.optJSONObject(i);
            if (figure == null) continue;
            final String key = figure.optString("key", "");
            String error = jsonString(figure, "bindingError");
            String reference = jsonString(figure, "reference");
            String detail = error.isEmpty() ? reference : error;
            TextView button = shortcut(figure.optString("caption", "Figure")
                    + (detail.isEmpty() ? "" : " · " + detail), new Runnable() {
                @Override public void run() {
                    if (holder[0] != null) holder[0].dismiss();
                    rebindSession(entry, key);
                }
            });
            rows.addView(button);
        }
        holder[0] = new AlertDialog.Builder(this, android.R.style.Theme_Material_Dialog_Alert)
                .setTitle("Rebind session").setView(scroll)
                .setNegativeButton("Cancel", null).show();
    }

    private void rebindSession(SessionCatalog.Entry entry, String key) {
        String json = DocBridge.rebindTerminalFigure(handle,
                editor.getText().toString(), key, entry.reference);
        try {
            JSONObject result = new JSONObject(json);
            if (result.has("error")) {
                fileMessage("Rebind refused: " + result.optString("error"));
                return;
            }
            applyState(json, true);
            setStatus("Rebound " + entry.caption + " to the selected figure");
        } catch (Exception e) {
            fileMessage("Rebind failed: " + e.getMessage());
        }
    }

    private void goToFigure(RenderedPage page, JSONObject figure) {
        float scale = page.pointScale();
        int y = Math.max(0, page.getTop()
                + Math.round((float) figure.optDouble("y", 0) * scale) - dp(8));
        previewScroll.scrollTo(0, y);
        page.requestFocus();
    }

    private boolean goToSession(SessionCatalog.Entry entry) {
        for (RenderedPage page : renderedPages) {
            for (int i = 0; i < page.figures.length(); i++) {
                JSONObject figure = page.figures.optJSONObject(i);
                if (figure == null || !entry.reference.equals(jsonString(figure, "reference"))) {
                    continue;
                }
                goToFigure(page, figure);
                return true;
            }
        }
        return false;
    }

    private void showNavigate(int tab) {
        if (editor == null || handle == 0) return;
        lastNavigateTab = tab;
        refreshSemantics(editor.getText().toString());
        final int shownTab = tab;
        ScrollView scroll = new ScrollView(this);
        LinearLayout rows = new LinearLayout(this);
        rows.setOrientation(LinearLayout.VERTICAL);
        rows.setPadding(dp(12), dp(8), dp(12), dp(12));
        scroll.addView(rows);

        LinearLayout tabs = new LinearLayout(this);
        tabs.setOrientation(LinearLayout.HORIZONTAL);
        String[] labels = {"Sections", "Definitions", "References"};
        for (int i = 0; i < labels.length; i++) {
            final int which = i;
            TextView button = shortcut((i == shownTab ? "• " : "") + labels[i], new Runnable() {
                @Override public void run() { showNavigate(which); }
            });
            button.setEnabled(i != shownTab);
            tabs.addView(button);
        }
        rows.addView(tabs);
        if (shownTab == NAV_REFERENCES) {
            final String text = editor.getText().toString();
            JSONArray references;
            try {
                references = new JSONArray(DocBridge.citationIndex(text, true));
            } catch (Exception e) {
                references = documentCitations;
            }
            final JSONArray citations = references;
            final EditText filter = new EditText(this);
            filter.setHint("Filter citations");
            filter.setSingleLine(true);
            rows.addView(filter);
            final LinearLayout referenceRows = new LinearLayout(this);
            referenceRows.setOrientation(LinearLayout.VERTICAL);
            rows.addView(referenceRows);
            final Runnable refresh = new Runnable() {
                @Override public void run() {
                    appendReferenceRows(referenceRows, citations, filter.getText().toString());
                }
            };
            filter.addTextChangedListener(new TextWatcher() {
                @Override public void beforeTextChanged(CharSequence s, int a, int b, int c) {}
                @Override public void onTextChanged(CharSequence s, int a, int b, int c) {}
                @Override public void afterTextChanged(Editable s) { refresh.run(); }
            });
            refresh.run();
        } else {
            appendNavigationRows(rows, shownTab);
        }

        new AlertDialog.Builder(this, android.R.style.Theme_Material_Dialog_Alert)
                .setTitle("Navigate").setView(scroll).setNegativeButton("Close", null).show();
    }

    private void appendReferenceRows(LinearLayout rows, JSONArray citations, String filter) {
        rows.removeAllViews();
        if (citations == null || citations.length() == 0) {
            rows.addView(panelLabel("No references"));
            return;
        }
        boolean any = false;
        for (int i = 0; i < citations.length(); i++) {
            JSONObject reference = citations.optJSONObject(i);
            if (reference == null) continue;
            JSONArray numbers = reference.optJSONArray("numbers");
            if (numbers == null || numbers.length() == 0) continue;
            int number = numbers.optInt(0);
            String label = reference.optString("label", "Reference");
            String snippet = reference.optString("source");
            String keys = citationKeyText(reference.optJSONArray("keys"));
            boolean resolved = reference.optBoolean("resolved", true);
            if (!CitationController.matchesFilter(filter, Integer.toString(number),
                    label, snippet, keys, resolved)) continue;
            any = true;
            TextView button = shortcut(citationHeading(number, label) + " · "
                    + (resolved ? "resolved" : "unresolved")
                    + (snippet.isEmpty() ? "" : "\n" + snippet), new Runnable() {
                @Override public void run() { showCitations(number); }
            });
            button.setContentDescription((resolved ? "Resolved" : "Unresolved")
                    + " citation [" + number + "] " + label);
            rows.addView(button);
        }
        if (!any) rows.addView(panelLabel("No matching references"));
    }

    private void appendNavigationRows(LinearLayout rows, int tab) {
        if (tab == NAV_SECTIONS) {
            JSONArray sections = documentSemantics.optJSONArray("sections");
            if (sections == null || sections.length() == 0) {
                rows.addView(panelLabel("No sections"));
                return;
            }
            for (int i = 0; i < sections.length(); i++) {
                final JSONObject section = sections.optJSONObject(i);
                if (section == null) continue;
                int start = section.optInt("start"), end = section.optInt("end");
                String label = section.optString("heading", "Section")
                        + " · " + section.optInt("statementCount") + " statements";
                rows.addView(navigationButton(label, "section",
                        section.optString("heading", "Section"), start, end, false));
            }
            return;
        }
        if (tab == NAV_DEFINITIONS) {
            JSONArray defs = documentSemantics.optJSONArray("defs");
            if (defs == null || defs.length() == 0) {
                rows.addView(panelLabel("No definitions"));
                return;
            }
            for (int i = 0; i < defs.length(); i++) {
                final JSONObject def = defs.optJSONObject(i);
                if (def == null) continue;
                JSONArray span = def.optJSONArray("span");
                int start = span == null ? 0 : span.optInt(0);
                int end = span == null ? start : span.optInt(1);
                String name = def.optString("name", "definition");
                rows.addView(navigationButton(name + " · " + def.optInt("useCount") + " uses",
                        "definition", name, start, end, true));
                if (def.optInt("useCount") > 0) {
                    rows.addView(shortcut("Uses of " + name, new Runnable() {
                        @Override public void run() { showUses(def); }
                    }));
                }
            }
            return;
        }
        if (documentCitations.length() == 0) {
            rows.addView(panelLabel("No references"));
            return;
        }
        for (int i = 0; i < documentCitations.length(); i++) {
            JSONObject reference = documentCitations.optJSONObject(i);
            if (reference == null) continue;
            int start = reference.optInt("start"), end = reference.optInt("end");
            rows.addView(navigationButton(reference.optString("label", "Reference")
                    + " · " + reference.optString("label"), "reference",
                    reference.optString("label", "Reference"), start, end, true));
        }
    }

    private TextView navigationButton(String label, final String kind, final String name,
                                      final int start, final int end, final boolean source) {
        TextView button = shortcut(label, new Runnable() {
            @Override public void run() { navigateTo(kind, name, start, end, source); }
        });
        button.setContentDescription(label + ". Open " + kind);
        return button;
    }

    private void showUses(JSONObject def) {
        final JSONArray uses = def.optJSONArray("uses");
        if (uses == null || uses.length() == 0) return;
        ScrollView scroll = new ScrollView(this);
        LinearLayout rows = new LinearLayout(this);
        rows.setOrientation(LinearLayout.VERTICAL);
        scroll.addView(rows);
        final String text = editor.getText().toString();
        for (int i = 0; i < uses.length(); i++) {
            final JSONObject use = uses.optJSONObject(i);
            if (use == null) continue;
            final int start = use.optInt("start"), end = use.optInt("end");
            String snippet = text.substring(Math.max(0, Math.min(start, text.length())),
                    Math.max(0, Math.min(end, text.length()))).trim();
            if (snippet.isEmpty()) snippet = "use " + (i + 1);
            rows.addView(shortcut(snippet + " · offset " + start, new Runnable() {
                @Override public void run() {
                    navigateTo("definition", "Definition use", start, end, true);
                }
            }));
        }
        new AlertDialog.Builder(this, android.R.style.Theme_Material_Dialog_Alert)
                .setTitle("Uses of " + def.optString("name", "definition"))
                .setView(scroll).setNegativeButton("Close", null).show();
    }

    private void showCitations(int openNumber) {
        if (editor == null || handle == 0) return;
        final PadSession.Owner dialogOwner =
                session.capture(PadSession.CITATION, "citations");
        final String text = editor.getText().toString();
        final int originCaret = editor.getSelectionStart();
        final int originScroll = previewScroll == null ? 0 : previewScroll.getScrollY();
        final boolean[] navigated = {false};
        final int[] currentOpen = {openNumber};
        try {
            JSONObject choices = new JSONObject(DocBridge.citationChoices());
            JSONArray styleArray = choices.optJSONArray("styles");
            JSONArray localeArray = choices.optJSONArray("locales");
            String[] styles = new String[styleArray == null ? 0 : styleArray.length()];
            String[] locales = new String[localeArray == null ? 0 : localeArray.length()];
            for (int i = 0; i < styles.length; i++) styles[i] = styleArray.optString(i);
            for (int i = 0; i < locales.length; i++) locales[i] = localeArray.optString(i);
            citationController.setChoices(styles, locales,
                    choices.optString("defaultStyle"), choices.optString("defaultLocale"));
            SharedPreferences prefs = getSharedPreferences(PREFS, MODE_PRIVATE);
            citationController.setSavedFormat(
                    prefs.getString(citationFormatKey("Style"), null),
                    prefs.getString(citationFormatKey("Locale"), null));
        } catch (Exception e) {
            citationController.setChoices(new String[0], new String[0], "apa", "en-US");
            citationController.setSavedFormat(null, null);
        }

        LinearLayout content = new LinearLayout(this);
        content.setOrientation(LinearLayout.VERTICAL);
        content.setPadding(dp(12), dp(8), dp(12), dp(12));
        LinearLayout controls = new LinearLayout(this);
        controls.setOrientation(LinearLayout.HORIZONTAL);
        final TextView styleLabel = panelLabel("Style");
        controls.addView(styleLabel);
        final Spinner styleSpinner = new Spinner(this);
        final Spinner localeSpinner = new Spinner(this);
        List<String> styleNames = new ArrayList<>();
        styleNames.add("Document default");
        Collections.addAll(styleNames, citationController.styles());
        ArrayAdapter<String> styleAdapter = new ArrayAdapter<>(this,
                android.R.layout.simple_spinner_item, styleNames);
        styleAdapter.setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item);
        styleSpinner.setAdapter(styleAdapter);
        List<String> localeNames = new ArrayList<>();
        localeNames.add("Default (" + citationController.defaultLocale() + ")");
        Collections.addAll(localeNames, citationController.locales());
        ArrayAdapter<String> localeAdapter = new ArrayAdapter<>(this,
                android.R.layout.simple_spinner_item, localeNames);
        localeAdapter.setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item);
        localeSpinner.setAdapter(localeAdapter);
        controls.addView(styleSpinner, new LinearLayout.LayoutParams(0,
                LinearLayout.LayoutParams.WRAP_CONTENT, 1f));
        controls.addView(localeSpinner, new LinearLayout.LayoutParams(0,
                LinearLayout.LayoutParams.WRAP_CONTENT, 1f));
        content.addView(controls);

        final EditText filter = new EditText(this);
        filter.setHint("Filter citations");
        filter.setSingleLine(true);
        content.addView(filter);
        final LinearLayout rows = new LinearLayout(this);
        rows.setOrientation(LinearLayout.VERTICAL);
        content.addView(rows);
        ScrollView scroll = new ScrollView(this);
        scroll.addView(content);
        final AlertDialog dialog = new AlertDialog.Builder(this, android.R.style.Theme_Material_Dialog_Alert)
                .setTitle("Citations").setView(scroll)
                .setNegativeButton("Close", null).create();
        final Runnable refresh = new Runnable() {
            @Override public void run() {
                rows.removeAllViews();
                appendCitationRows(rows, text, dialog, dialogOwner, 0,
                        currentOpen[0], Collections.<CitationController.Path>emptyList(), true,
                        navigated);
            }
        };
        filter.addTextChangedListener(new TextWatcher() {
            @Override public void beforeTextChanged(CharSequence s, int a, int b, int c) {}
            @Override public void onTextChanged(CharSequence s, int a, int b, int c) {}
            @Override public void afterTextChanged(Editable s) {
                citationController.setFilter(s.toString());
                refresh.run();
            }
        });
        styleSpinner.setSelection(citationIndex(citationController.styles(), citationController.style()) + 1);
        localeSpinner.setSelection(citationIndex(citationController.locales(), citationController.locale()) + 1);
        styleSpinner.setOnItemSelectedListener(new AdapterView.OnItemSelectedListener() {
            @Override public void onItemSelected(AdapterView<?> parent, View view, int position, long id) {
                citationController.setStyle(position == 0 ? null : citationController.styles()[position - 1]);
                saveCitationFormat();
                refresh.run();
            }
            @Override public void onNothingSelected(AdapterView<?> parent) {}
        });
        localeSpinner.setOnItemSelectedListener(new AdapterView.OnItemSelectedListener() {
            @Override public void onItemSelected(AdapterView<?> parent, View view, int position, long id) {
                citationController.setLocale(position == 0 ? null : citationController.locales()[position - 1]);
                saveCitationFormat();
                refresh.run();
            }
            @Override public void onNothingSelected(AdapterView<?> parent) {}
        });
        TextView reset = shortcut("Reset style and locale", new Runnable() {
            @Override public void run() {
                citationController.resetFormat();
                saveCitationFormat();
                styleSpinner.setSelection(0);
                localeSpinner.setSelection(0);
                refresh.run();
            }
        });
        content.addView(reset);
        dialog.setOnDismissListener(new DialogInterface.OnDismissListener() {
            @Override public void onDismiss(DialogInterface ignored) {
                if (navigated[0]) return;
                if (editor != null && originCaret >= 0 && originCaret <= editor.length()) {
                    clearSelectionAnchor();
                    editor.setSelection(originCaret);
                }
                if (previewScroll != null) {
                    previewScroll.scrollTo(0, originScroll);
                    previewScroll.requestFocus();
                }
            }
        });
        refresh.run();
        dialog.show();
    }

    private String citationFormatKey(String suffix) {
        return "citation" + suffix + ":" + session.documentId();
    }

    private void saveCitationFormat() {
        SharedPreferences.Editor edit = getSharedPreferences(PREFS, MODE_PRIVATE).edit();
        if (citationController.style() == null) edit.remove(citationFormatKey("Style"));
        else edit.putString(citationFormatKey("Style"), citationController.style());
        if (citationController.locale() == null) edit.remove(citationFormatKey("Locale"));
        else edit.putString(citationFormatKey("Locale"), citationController.locale());
        edit.apply();
    }

    private static int citationIndex(String[] values, String selected) {
        if (selected == null) return -1;
        for (int i = 0; i < values.length; i++) if (values[i].equals(selected)) return i;
        return -1;
    }

    private static String citationHeading(int number, String label) {
        String value = label == null ? "" : label.trim();
        if (value.matches("^\\[\\d+\\].*")) return value;
        return "[" + number + "]" + (value.isEmpty() ? "" : " " + value);
    }

    private static String citationKeyText(JSONArray keys) {
        if (keys == null) return "";
        StringBuilder text = new StringBuilder();
        for (int i = 0; i < keys.length(); i++) {
            if (text.length() > 0) text.append(", ");
            text.append(keys.optString(i));
        }
        return text.toString();
    }

    private void appendCitationRows(LinearLayout rows, final String scope, final AlertDialog dialog,
                                    final PadSession.Owner dialogOwner,
                                    final int depth, int openNumber,
                                    final List<CitationController.Path> parentPath,
                                    boolean resolveStatus, final boolean[] navigated) {
        try {
            JSONArray citations = new JSONArray(DocBridge.citationIndex(scope, resolveStatus));
            if (citations.length() == 0 && depth == 0) rows.addView(panelLabel("No citations"));
            for (int i = 0; i < citations.length(); i++) {
                JSONObject cite = citations.getJSONObject(i);
                JSONArray numbers = cite.getJSONArray("numbers");
                String label = cite.optString("label");
                String snippet = cite.optString("source");
                String keys = citationKeyText(cite.optJSONArray("keys"));
                boolean resolved = cite.optBoolean("resolved", true);
                for (int j = 0; j < numbers.length(); j++) {
                    final int number = numbers.getInt(j);
                    final int start = cite.optInt("start");
                    final int end = cite.optInt("end");
                    final boolean visible = citationController.matches(
                            Integer.toString(number), label, snippet, keys, resolved);
                    final List<CitationController.Path> path = citationController.append(
                            parentPath, scope, number, start, end, label);
                    final boolean repeated = path == null
                            && CitationController.isCycle(parentPath, scope, number, start, end);
                    final boolean tooDeep = path == null && !repeated;
                    final LinearLayout item = new LinearLayout(this);
                    item.setOrientation(LinearLayout.VERTICAL);
                    item.setVisibility(visible ? View.VISIBLE : View.GONE);
                    final TextView heading = panelLabel("");
                    String title = (path == null ? (repeated ? "↻ " : "… ") : "▸ ")
                            + citationHeading(number, label);
                    String status = resolved ? "resolved" : "unresolved";
                    heading.setText(title + "\n" + (snippet.isEmpty() ? status : snippet + " · " + status));
                    heading.setTextColor(Color.parseColor("#B7D5EA"));
                    heading.setPadding(dp(8 + depth * 12), dp(8), dp(8), dp(8));
                    final LinearLayout details = new LinearLayout(this);
                    details.setOrientation(LinearLayout.VERTICAL);
                    details.setPadding(dp(16 + depth * 12), 0, dp(8), dp(8));
                    details.setVisibility(View.GONE);
                    final boolean[] loaded = {false};
                    heading.setOnClickListener(new View.OnClickListener() {
                        @Override public void onClick(View v) {
                            if (repeated) {
                                goToCitationSource(dialog, navigated, label, parentPath);
                                return;
                            }
                            if (tooDeep) {
                                goToCitationSource(dialog, navigated, label, parentPath);
                                return;
                            }
                            boolean open = details.getVisibility() != View.VISIBLE;
                            details.setVisibility(open ? View.VISIBLE : View.GONE);
                            heading.setText((open ? "▾" : "▸") + heading.getText().toString().substring(1));
                            if (open && !loaded[0]) {
                                loaded[0] = true;
                                loadCitationDetail(details, heading, scope, number, dialog,
                                        dialogOwner, depth, path, label, navigated);
                            }
                        }
                    });
                    item.addView(heading);
                    item.addView(details);
                    rows.addView(item);
                    if (number == openNumber && visible) heading.performClick();
                }
            }
        } catch (Exception e) { rows.addView(panelLabel("Could not list citations")); }
    }

    private void goToCitationSource(AlertDialog dialog, boolean[] navigated,
                                    String label, List<CitationController.Path> path) {
        int[] source = CitationController.rootSource(path);
        if (source == null || editor == null) return;
        navigated[0] = true;
        dialog.dismiss();
        navigateTo("citation", label == null || label.isEmpty() ? "Citation" : label,
                source[0], source[1], true);
    }

    private void loadCitationDetail(final LinearLayout details, final TextView heading,
                                    final String scope, final int number,
                                    final AlertDialog dialog, final PadSession.Owner dialogOwner,
                                    final int depth, final List<CitationController.Path> path,
                                    final String label, final boolean[] navigated) {
        details.addView(panelLabel("Loading…"));
        final float scale = Math.max(1f, Math.min(3f, getResources().getDisplayMetrics().density));
        final float width = Math.max(60f, (getResources().getDisplayMetrics().widthPixels - dp(100)) / scale);
        final int version = citationController.version();
        final String style = citationController.style() == null ? "" : citationController.style();
        final String locale = citationController.locale() == null ? "" : citationController.locale();
        renderWork.submit("citation", RenderController.EXPLICIT, new RenderController.Job() {
            @Override public void run() {
                final String result = DocBridge.citationDetail(scope, number, width, scale, style, locale);
                main.post(new Runnable() {
                    @Override public void run() {
                        if (!dialog.isShowing() || !session.sameDocument(dialogOwner)
                                || version != citationController.version()
                                || details.getParent() == null) return;
                        details.removeAllViews();
                        try {
                            JSONObject detail = new JSONObject(result);
                            TextView breadcrumb = panelLabel(CitationController.breadcrumb(path));
                            breadcrumb.setTextColor(Color.parseColor("#D8E8F4"));
                            details.addView(breadcrumb);
                            if (depth > 0) {
                                details.addView(shortcut("Back to previous citation", new Runnable() {
                                    @Override public void run() {
                                        details.setVisibility(View.GONE);
                                        String text = heading.getText().toString();
                                        if (text.startsWith("▾")) heading.setText("▸" + text.substring(1));
                                    }
                                }));
                            }
                            if (detail.has("error")) {
                                details.addView(panelLabel(detail.getString("error")));
                                JSONObject meta = detail.optJSONObject("metadata");
                                if (meta != null) details.addView(panelLabel("Source: " + meta.optString("source")));
                            } else if ("bibliography".equals(detail.optString("kind"))) {
                                details.addView(panelLabel(detail.optString("text")));
                                JSONArray entries = detail.optJSONArray("entries");
                                if (entries != null) {
                                    for (int k = 0; k < entries.length(); k++)
                                        details.addView(panelLabel(entries.optString(k)));
                                }
                                if (!detail.optBoolean("resolved"))
                                    details.addView(panelLabel(detail.optString("errorDetail", "Citation could not be resolved")));
                                JSONObject meta = detail.optJSONObject("metadata");
                                if (meta != null && !meta.optString("source").isEmpty()) {
                                    details.addView(panelLabel("Source: " + meta.optString("source")));
                                    details.addView(shortcut("Edit source", new Runnable() {
                                        @Override public void run() {
                                            goToCitationSource(dialog, navigated, label, path);
                                        }
                                    }));
                                }
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
                                if (path.size() < CitationController.MAX_DEPTH) {
                                    appendCitationRows(details, detail.optString("text"), dialog,
                                            dialogOwner, depth + 1, 0, path, false, navigated);
                                } else {
                                    details.addView(panelLabel("Depth limit — use the original citation source"));
                                }
                            }
                            int[] root = CitationController.rootSource(path);
                            if (root != null && !detail.has("error")) {
                                details.addView(shortcut("Go to source", new Runnable() {
                                    @Override public void run() {
                                        goToCitationSource(dialog, navigated, label, path);
                                    }
                                }));
                            }
                        } catch (Exception e) { details.addView(panelLabel("Could not display citation")); }
                    }
                });
            }
        });
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

    /**
     * Suggest marker names only for the pair being edited. The scan is
     * already available; this never enumerates all possible pairs.
     */
    private void updateTargetSuggestions(final EditText field) {
        if (field != targetsField || modifierSuggestions == null) return;
        modifierSuggestions.removeAllViews();
        if (!field.hasFocus()) return;
        String value = field.getText().toString();
        int caret = Math.max(0, Math.min(field.getSelectionStart(), value.length()));
        int segmentStart = value.lastIndexOf(',', Math.max(0, caret - 1)) + 1;
        int segmentEnd = value.indexOf(',', caret);
        if (segmentEnd < 0) segmentEnd = value.length();
        String segment = value.substring(segmentStart, segmentEnd);
        int leading = 0;
        while (leading < segment.length() && Character.isWhitespace(segment.charAt(leading))) leading++;
        int space = -1;
        for (int i = leading; i < segment.length(); i++) {
            if (Character.isWhitespace(segment.charAt(i))) { space = i; break; }
        }
        final boolean endToken = space >= 0;
        String first = (endToken ? segment.substring(leading, space) : segment.substring(leading)).trim();
        if (first.startsWith("#")) first = first.substring(1);
        String query = first;
        int startIndex = -1;
        if (endToken) {
            String rest = segment.substring(space).trim();
            if (rest.startsWith("#")) rest = rest.substring(1);
            query = rest;
            for (int i = 0; i < documentMarkers.length(); i++) {
                if (first.equals(documentMarkers.optJSONObject(i).optString("id"))) { startIndex = i; break; }
            }
            if (startIndex < 0) return;
        }
        int shown = 0;
        for (int i = endToken ? startIndex + 1 : 0;
                i < documentMarkers.length() && shown < 6; i++) {
            JSONObject marker = documentMarkers.optJSONObject(i);
            if (marker == null) continue;
            String id = marker.optString("id");
            if (!id.startsWith(query)) continue;
            final String chosen = id;
            String snippet = markerSnippet(marker);
            TextView suggestion = shortcut(id + (snippet.isEmpty() ? "" : " · " + snippet),
                    new Runnable() {
                @Override public void run() {
                    applyTargetSuggestion(field, chosen, endToken);
                }
            });
            suggestion.setContentDescription("Suggest marker " + id + (snippet.isEmpty() ? "" : ", " + snippet));
            modifierSuggestions.addView(suggestion);
            shown++;
        }
    }

    private String markerSnippet(JSONObject marker) {
        if (editor == null) return "";
        String text = editor.getText().toString();
        int end = marker.optInt("end", 0);
        int next = text.length();
        for (int i = 0; i < documentMarkers.length(); i++) {
            JSONObject other = documentMarkers.optJSONObject(i);
            if (other == null) continue;
            int start = other.optInt("start", 0);
            if (start >= end && start < next) next = start;
        }
        String snippet = text.substring(Math.max(0, Math.min(end, text.length())),
                Math.max(0, Math.min(next, text.length()))).trim().replace('\n', ' ');
        if (snippet.length() > 32) snippet = snippet.substring(0, 32) + "…";
        return snippet;
    }

    private void applyTargetSuggestion(EditText field, String id, boolean endToken) {
        if (field != targetsField) return;
        String value = field.getText().toString();
        int caret = Math.max(0, Math.min(field.getSelectionStart(), value.length()));
        int segmentStart = value.lastIndexOf(',', Math.max(0, caret - 1)) + 1;
        int segmentEnd = value.indexOf(',', caret);
        if (segmentEnd < 0) segmentEnd = value.length();
        String segment = value.substring(segmentStart, segmentEnd);
        int leading = 0;
        while (leading < segment.length() && Character.isWhitespace(segment.charAt(leading))) leading++;
        int space = -1;
        for (int i = leading; i < segment.length(); i++) {
            if (Character.isWhitespace(segment.charAt(i))) { space = i; break; }
        }
        String replacement;
        if (!endToken) {
            String rest = space < 0 ? "" : segment.substring(space);
            replacement = segment.substring(0, leading) + id + " " + rest.trim();
        } else {
            replacement = segment.substring(0, space + 1) + id;
        }
        field.getText().replace(segmentStart, segmentEnd, replacement);
        field.setSelection(Math.min(field.length(), segmentStart + replacement.length()));
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
        java.util.LinkedHashSet<String> complete = new java.util.LinkedHashSet<>();
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
            previewScroll.setVisibility(View.VISIBLE);
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
        boolean restoreSourceFocus = sourceField != null && sourceField.hasFocus();
        modifierPanel.removeAllViews();
        modifierPanel.setVisibility(View.VISIBLE);
        modifierPanel.addView(shortcut("Show document", new Runnable() {
            @Override public void run() { closeInspector(); }
        }));

        final boolean wideInspector = wideInspector();
        LinearLayout controls = new LinearLayout(this);
        controls.setOrientation(wideInspector ? LinearLayout.HORIZONTAL : LinearLayout.VERTICAL);
        modifierPanel.addView(controls);
        mathHint = panelLabel("");
        mathHint.setVisibility(View.GONE);
        modifierPanel.addView(mathHint);

        final JSONObject definition = definitionAt(editor.getSelectionStart());
        if (definition != null) {
            LinearLayout actions = new LinearLayout(this);
            actions.setOrientation(LinearLayout.HORIZONTAL);
            final JSONArray defSpan = definition.optJSONArray("span");
            final int defStart = defSpan == null ? 0 : defSpan.optInt(0);
            final int defEnd = defSpan == null ? defStart : defSpan.optInt(1);
            actions.addView(shortcut("Go to definition", new Runnable() {
                @Override public void run() {
                    navigateTo("definition", definition.optString("name", "Definition"),
                            defStart, defEnd, true);
                }
            }));
            actions.addView(shortcut("Find uses", new Runnable() {
                @Override public void run() { showUses(definition); }
            }));
            actions.addView(shortcut("Rename definition…", new Runnable() {
                @Override public void run() { showRenameDialog(definition); }
            }));
            modifierPanel.addView(actions);
        }

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
                wideInspector ? 0 : LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.WRAP_CONTENT, wideInspector ? 1f : 0f));
        updateSourceMarkerVisibility();
        if (restoreSourceFocus) focusSourceCaret();

        if (!markedPart) {
            targetsField = null;
            modifierRows = null;
            modifierSuggestions = null;
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
                wideInspector ? 0 : LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.WRAP_CONTENT, wideInspector ? 1f : 0f));

        modifierSuggestions = new LinearLayout(this);
        modifierSuggestions.setOrientation(LinearLayout.VERTICAL);
        modifierPanel.addView(modifierSuggestions);
        updateTargetSuggestions(targets);
        targets.addTextChangedListener(new TextWatcher() {
            @Override public void beforeTextChanged(CharSequence s, int start, int count, int after) {}
            @Override public void onTextChanged(CharSequence s, int start, int before, int count) {}
            @Override public void afterTextChanged(Editable s) {
                updateTargetSuggestions(targets);
                scheduleTargetCommit(targets);
            }
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

    /** Preview a checked semantic rename, then apply it as one grouped edit. */
    private void showRenameDialog(final JSONObject definition) {
        if (handle == 0) return;
        final int defIndex = definition.optInt("index", -1);
        final String oldName = definition.optString("name", "");
        if (defIndex < 0 || oldName.isEmpty()) return;
        final EditText input = new EditText(this);
        input.setSingleLine(true);
        input.setText(oldName);
        input.setSelection(0, input.length());
        input.setHint("One math identifier");
        input.setContentDescription("New definition name");
        input.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS);
        final AlertDialog[] holder = new AlertDialog[1];
        holder[0] = new AlertDialog.Builder(this, android.R.style.Theme_Material_Dialog_Alert)
                .setTitle("Rename definition")
                .setMessage("Preview the definition and every resolved use before applying.")
                .setView(input)
                .setPositiveButton("Preview", null)
                .setNegativeButton("Cancel", null)
                .create();
        holder[0].setOnShowListener(new DialogInterface.OnShowListener() {
            @Override public void onShow(DialogInterface dialog) {
                holder[0].getButton(AlertDialog.BUTTON_POSITIVE)
                        .setOnClickListener(new View.OnClickListener() {
                    @Override public void onClick(View v) {
                        String name = input.getText().toString().trim();
                        String preview = DocBridge.renamePreview(
                                editor.getText().toString(), defIndex, name);
                        try {
                            JSONObject result = new JSONObject(preview);
                            if (result.has("error")) {
                                input.setError(result.getString("error"));
                                return;
                            }
                            holder[0].dismiss();
                            confirmRename(defIndex, result.optString("name", name),
                                    result.optInt("uses"));
                        } catch (Exception e) {
                            input.setError("Could not preview this rename");
                        }
                    }
                });
            }
        });
        holder[0].show();
    }

    private void confirmRename(final int defIndex, final String newName, int uses) {
        new AlertDialog.Builder(this, android.R.style.Theme_Material_Dialog_Alert)
                .setTitle("Rename to " + newName + "?")
                .setMessage("Renames the definition and " + uses
                        + " resolved use" + (uses == 1 ? "" : "s") + ".")
                .setPositiveButton("Rename", new DialogInterface.OnClickListener() {
                    @Override public void onClick(DialogInterface dialog, int which) {
                        String expected = editor.getText().toString();
                        String state = DocBridge.renameDefinition(handle, expected, defIndex, newName);
                        try {
                            JSONObject result = new JSONObject(state);
                            if (result.has("error")) {
                                fileMessage("Rename refused: " + result.getString("error"));
                                return;
                            }
                            applyState(state, true);
                            setStatus("Renamed definition to " + newName);
                            if (inspectorOpen) {
                                inspectorSignature = "";
                                updateModifierPanel();
                                focusSourceCaret();
                            }
                        } catch (Exception e) {
                            fileMessage("Rename failed: " + e.getMessage());
                        }
                    }
                })
                .setNegativeButton("Cancel", null)
                .show();
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
        modifierSuggestions = null;
        expandedModifierRow = null;
        modifierRowGeneration++;
        inspectorSignature = "";
        session.invalidate(PadSession.INSPECTOR);
        main.removeCallbacks(updateModifierComparisons);
        modifierPanel.setVisibility(View.GONE);
        previewScroll.setVisibility(View.VISIBLE);
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
        renderWork.submit("inspector", RenderController.EXPLICIT, new RenderController.Job() {
            @Override public void run() {
                final String result = DocBridge.modifierRelationships(text, snapshot, start, end);
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

    private boolean wideInspector() {
        int width = modifierPanel == null ? 0 : modifierPanel.getWidth();
        float density = getResources().getDisplayMetrics().density;
        if (width <= 0) width = getResources().getDisplayMetrics().widthPixels;
        return InspectorView.columns(width, density);
    }

    private void comparisonRow(View left, View right) {
        LinearLayout row = new LinearLayout(this);
        row.setOrientation(wideInspector() ? LinearLayout.HORIZONTAL : LinearLayout.VERTICAL);
        boolean wide = wideInspector();
        row.addView(left, new LinearLayout.LayoutParams(wide ? 0 : LinearLayout.LayoutParams.MATCH_PARENT,
                left instanceof ImageView ? (Integer) left.getTag() : LinearLayout.LayoutParams.WRAP_CONTENT, wide ? 1f : 0f));
        row.addView(right, new LinearLayout.LayoutParams(wide ? 0 : LinearLayout.LayoutParams.MATCH_PARENT,
                right instanceof ImageView ? (Integer) right.getTag() : LinearLayout.LayoutParams.WRAP_CONTENT, wide ? 1f : 0f));
        modifierRows.addView(row);
    }

    private void showModifierComparisons(String json) {
        modifierRows.removeAllViews();
        expandedModifierRow = null;
        try {
            JSONObject result = new JSONObject(json);
            if (result.has("error")) { modifierRows.addView(panelLabel(result.getString("error"))); return; }
            JSONArray incoming = result.getJSONArray("incoming");
            modifierRows.addView(panelLabel("Incoming modifiers"));
            if (incoming.length() == 0) modifierRows.addView(panelLabel("None"));
            for (int i = 0; i < incoming.length(); i++) {
                addModifierRelationship(incoming.getJSONObject(i), false);
            }
            JSONArray outgoing = result.getJSONArray("outgoing");
            modifierRows.addView(panelLabel("Outgoing targets"));
            if (outgoing.length() == 0) modifierRows.addView(panelLabel("No applications yet"));
            for (int i = 0; i < outgoing.length(); i++) {
                addModifierRelationship(outgoing.getJSONObject(i), true);
            }
        } catch (Exception e) {
            modifierRows.addView(panelLabel("Could not display comparisons"));
        }
    }

    private void addModifierRelationship(final JSONObject row, final boolean outgoing) {
        String snippet = row.optString("snippet");
        String label = outgoing ? snippet : row.optString("label");
        String ids = citationKeyText(row.optJSONArray("targetIds"));
        boolean resolved = row.optBoolean("resolved", true);
        String title = (outgoing ? "→ " : "← ") + (label.isEmpty() ? "(empty)" : label)
                + (ids.isEmpty() ? "" : "  #" + ids.replace(", ", " #"))
                + " · " + (resolved ? "resolved" : "unresolved");
        LinearLayout card = new LinearLayout(this);
        card.setOrientation(LinearLayout.VERTICAL);
        final TextView heading = panelLabel(title + (snippet.isEmpty() || outgoing ? "" : "\n" + snippet));
        heading.setTextColor(Color.parseColor("#B7D5EA"));
        final LinearLayout details = new LinearLayout(this);
        details.setOrientation(LinearLayout.VERTICAL);
        details.setVisibility(View.GONE);
        heading.setOnClickListener(new View.OnClickListener() {
            @Override public void onClick(View v) {
                if (expandedModifierRow != null && expandedModifierRow != details) {
                    expandedModifierRow.setVisibility(View.GONE);
                }
                boolean open = details.getVisibility() != View.VISIBLE;
                details.setVisibility(open ? View.VISIBLE : View.GONE);
                expandedModifierRow = open ? details : null;
                if (open && details.getChildCount() == 0) {
                    details.addView(panelLabel("Loading comparison…"));
                    requestModifierRow(details, row, outgoing);
                }
            }
        });
        card.addView(heading);
        card.addView(details);
        LinearLayout actions = new LinearLayout(this);
        actions.setOrientation(LinearLayout.HORIZONTAL);
        TextView modifier = shortcut("Go to modifier", new Runnable() {
            @Override public void run() {
                goToModifier(row, outgoing);
            }
        });
        boolean hasSource = row.has("sourceStart") && !row.isNull("sourceStart");
        modifier.setEnabled(outgoing || hasSource);
        actions.addView(modifier);
        actions.addView(shortcut("Go to target", new Runnable() {
            @Override public void run() {
                goToModifierTarget(row, outgoing);
            }
        }));
        actions.addView(shortcut("Remove link", new Runnable() {
            @Override public void run() { removeModifierRelationship(row, outgoing); }
        }));
        card.addView(actions);
        if (!resolved) card.addView(panelLabel(row.optString("error", "Unresolved relationship")));
        modifierRows.addView(card);
    }

    private void goToModifier(JSONObject row, boolean outgoing) {
        boolean hasSource = row.has("sourceStart") && !row.isNull("sourceStart");
        int start = outgoing ? modifierBodyStart : (hasSource ? row.optInt("sourceStart", -1) : -1);
        int end = outgoing ? modifierBodyEnd : (hasSource ? row.optInt("sourceEnd", -1) : -1);
        if (start < 0 || end < start) return;
        navigateTo("modifier", outgoing ? "Modifier" : row.optString("label", "Modifier"),
                start, end, true);
    }

    private void goToModifierTarget(JSONObject row, boolean outgoing) {
        int start = outgoing ? row.optInt("start", -1) : modifierBodyStart;
        int end = outgoing ? row.optInt("end", -1) : modifierBodyEnd;
        if (start < 0 || end < start) return;
        navigateTo("modifier-target", "Modified text", start, end, true);
    }

    private void removeModifierRelationship(JSONObject row, boolean outgoing) {
        String state;
        String rowStart = row.optString("startId");
        String rowEnd = row.optString("endId");
        JSONArray targetIds = row.optJSONArray("targetIds");
        if (outgoing) {
            if (targetIds != null && targetIds.length() >= 2) {
                String next = removePair(markerPairsFor(modifierStart, modifierEnd),
                        targetIds.optString(0), targetIds.optString(1));
                state = DocBridge.setMarkerTargets(handle, modifierStart, modifierEnd, next);
            } else {
                state = DocBridge.setModifier(handle, modifierStart, modifierEnd,
                        row.optInt("start"), row.optInt("end"), 0);
            }
        } else if (targetIds != null && targetIds.length() >= 2 && !rowStart.isEmpty()) {
            String next = removePair(markerPairsFor(rowStart, rowEnd), modifierStart, modifierEnd);
            state = DocBridge.setMarkerTargets(handle, rowStart, rowEnd, next);
        } else if (!rowStart.isEmpty()) {
            state = DocBridge.setModifier(handle, rowStart, rowEnd,
                    row.optInt("start"), row.optInt("end"), 0);
        } else {
            return;
        }
        try {
            JSONObject parsed = new JSONObject(state);
            if (parsed.has("error")) {
                setStatus("Remove refused: " + parsed.getString("error"));
                return;
            }
            applyState(state, false);
            inspectorSignature = "";
            updateModifierPanel();
        } catch (Exception e) {
            setStatus("Could not remove modifier link");
        }
    }

    private static String removePair(String list, String start, String end) {
        java.util.ArrayList<String> kept = new java.util.ArrayList<>();
        String wanted = start + " " + end;
        for (String item : list.split(",")) {
            String value = item.trim().replaceFirst("^#", "").replace(" #", " ");
            if (!value.isEmpty() && !value.equals(wanted)) kept.add(value);
        }
        return android.text.TextUtils.join(", ", kept);
    }

    private void requestModifierRow(final LinearLayout details, final JSONObject row, final boolean outgoing) {
        int start = outgoing ? row.optInt("start") : modifierBodyStart;
        int end = outgoing ? row.optInt("end") : modifierBodyEnd;
        String excludeStart = outgoing ? modifierStart : row.optString("startId");
        String excludeEnd = outgoing ? modifierEnd : row.optString("endId");
        final float scale = Math.max(1f, Math.min(3f, getResources().getDisplayMetrics().density));
        final float width = Math.max(60f, modifierRows.getWidth() / (wideInspector() ? 2f * scale : scale));
        final String key = InspectorView.ComparisonCache.key(modifierStart, modifierEnd,
                start, end, excludeStart, excludeEnd, (int) width, scale,
                editor.getText().toString().hashCode());
        Object cached = modifierRowCache.get(key);
        if (cached instanceof JSONObject) {
            renderModifierRow(details, (JSONObject) cached);
            return;
        }
        final String text = editor.getText().toString();
        final String snapshot = DocBridge.snapshot(handle);
        final int generation = ++modifierRowGeneration;
        renderWork.submit("modifier-row", RenderController.EXPLICIT, new RenderController.Job() {
            @Override public void run() {
                final String result = DocBridge.modifierComparisonRow(text, snapshot, start, end,
                        excludeStart, excludeEnd, width, scale);
                main.post(new Runnable() {
                    @Override public void run() {
                        if (generation != modifierRowGeneration || details.getParent() == null) return;
                        try {
                            JSONObject parsed = new JSONObject(result);
                            if (!parsed.has("error")) modifierRowCache.put(key, parsed);
                            renderModifierRow(details, parsed);
                        } catch (Exception e) {
                            details.removeAllViews();
                            details.addView(panelLabel("Could not display comparison"));
                        }
                    }
                });
            }
        });
    }

    private void renderModifierRow(LinearLayout details, JSONObject result) {
        details.removeAllViews();
        if (result.has("error")) {
            details.addView(panelLabel(result.optString("error", "Comparison unavailable")));
            return;
        }
        comparisonRow(panelLabel("With modifier"), comparisonImage(result.optJSONObject("with")));
        comparisonRow(panelLabel("Without modifier"), comparisonImage(result.optJSONObject("without")));
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
        clearDiagnostics();
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
                "caret", caret);
        initialPagePending = false;
        initialLocatePending = false;
        requestChunk(text, 0, caret, "initial");
    }

    /** One worker-decoded page; no bitmap work runs on the UI thread. */
    private static final class DecodedPage {
        final int index, width, height, visiblePixels;
        final double textTop, widthPt, heightPt;
        final boolean hasFigures;
        final JSONArray figures, hits;
        final Bitmap bitmap;

        DecodedPage(int index, int width, int height, int visiblePixels, double textTop,
                    double widthPt, double heightPt, boolean hasFigures,
                    JSONArray figures, JSONArray hits, Bitmap bitmap) {
            this.index = index;
            this.width = width;
            this.height = height;
            this.visiblePixels = visiblePixels;
            this.textTop = textTop;
            this.widthPt = widthPt;
            this.heightPt = heightPt;
            this.hasFigures = hasFigures;
            this.figures = figures == null ? new JSONArray() : figures;
            this.hits = hits;
            this.bitmap = bitmap;
        }
    }

    private static final class DecodedChunk {
        final String layoutId, error, intent;
        final int total, start, captionStart;
        final List<DecodedPage> pages;
        final JSONArray pageSizes, warnings, diagnostics;

        DecodedChunk(String layoutId, String error, int total, int start, int captionStart,
                     List<DecodedPage> pages, JSONArray pageSizes, JSONArray warnings,
                     JSONArray diagnostics, String intent) {
            this.layoutId = layoutId;
            this.error = error;
            this.total = total;
            this.start = start;
            this.captionStart = captionStart;
            this.pages = pages;
            this.pageSizes = pageSizes;
            this.warnings = warnings;
            this.diagnostics = diagnostics == null ? new JSONArray() : diagnostics;
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
            JSONArray diagnostics = prepared.optJSONArray("diagnostics");
            if (diagnostics == null) diagnostics = new JSONArray();
            String error = prepared.optString("error", "");
            if (!error.isEmpty()) {
                return new DecodedChunk(layoutId, error, 0, start, -1,
                        new ArrayList<DecodedPage>(), new JSONArray(), new JSONArray(),
                        diagnostics, intent);
            }
            int total = prepared.getInt("total");
            int caption = prepared.optInt("captionStart", -1);
            JSONArray pageSizes = prepared.optJSONArray("pages");
            if (pageSizes == null) pageSizes = new JSONArray();
            JSONArray warnings = prepared.optJSONArray("warnings");
            if (warnings == null) warnings = new JSONArray();
            if (start >= total) {
                return new DecodedChunk(layoutId, "Page is outside the layout", total, start,
                        caption, new ArrayList<DecodedPage>(), pageSizes, warnings,
                        diagnostics, intent);
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
                        new ArrayList<DecodedPage>(), pageSizes, warnings,
                        diagnostics, intent);
            }
            int w = p.getInt("width");
            int h = p.getInt("height");
            JSONArray hits = p.getJSONArray("hits");
            JSONArray figures = p.optJSONArray("figures");
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
                    p.optBoolean("hasFigures", false),
                    figures, hits, bmp));
            decodeMs += (System.nanoTime() - decodeStart) / 1_000_000;
            trace("renderCost", "layoutMs", layoutMs, "rasterMs", rasterMs,
                    "decodeMs", decodeMs, "pages", pages.size(), "layoutId", layoutId);
            return new DecodedChunk(layoutId, "", total, page, caption,
                    pages, pageSizes, warnings, diagnostics, intent);
        } catch (Exception e) {
            return new DecodedChunk(layoutId, "render: " + e.getMessage(), 0, start, -1,
                    new ArrayList<DecodedPage>(), new JSONArray(), new JSONArray(),
                    new JSONArray(), intent);
        }
    }

    /**
     * Shows a render diagnostic and stops the page stream until the next
     * edit (a half-typed expression must not retry per scroll tick).
     */
    private void showPreviewError(String message, JSONArray diagnostics) {
        if (diagnostics != null && diagnostics.length() > 0) setDiagnostics(diagnostics, renderLayoutId);
        else setDiagnostics(syntheticDiagnostic("render", message, "error"), renderLayoutId);
        previewError.setText(message);
        previewError.setVisibility(View.VISIBLE);
        totalPages = pagesLoaded;
    }

    private JSONArray syntheticDiagnostic(String code, String message, String severity) {
        JSONArray result = new JSONArray();
        try {
            JSONObject diagnostic = new JSONObject();
            diagnostic.put("id", renderLayoutId + ":" + code + ":runtime");
            diagnostic.put("severity", severity);
            diagnostic.put("code", code);
            diagnostic.put("message", message == null ? "Preview unavailable" : message);
            diagnostic.put("captureId", renderLayoutId);
            diagnostic.put("start", JSONObject.NULL);
            diagnostic.put("end", JSONObject.NULL);
            diagnostic.put("markerPair", JSONObject.NULL);
            diagnostic.put("provenance", "document");
            result.put(diagnostic);
        } catch (Exception ignored) { }
        return result;
    }

    /** Replace the visible diagnostic set for one accepted layout capture. */
    private void setDiagnostics(JSONArray diagnostics, String capture) {
        currentDiagnostics = diagnostics == null ? new JSONArray() : diagnostics;
        int errors = 0, warnings = 0;
        for (int i = 0; i < currentDiagnostics.length(); i++) {
            JSONObject diagnostic = currentDiagnostics.optJSONObject(i);
            if (diagnostic == null) continue;
            String severity = displayDiagnosticSeverity(diagnostic);
            if ("error".equals(severity)) errors++;
            else if ("warning".equals(severity)) warnings++;
        }
        diagnosticCount = errors + warnings;
        trace("diagnostics", "capture", capture, "count", diagnosticCount,
                "errors", errors, "warnings", warnings,
                "notes", currentDiagnostics.length() - diagnosticCount);
        setStatus(statusText);
        previewError.setOnClickListener(null);
        previewError.setClickable(false);
        if (diagnosticCount == 0) {
            previewError.setVisibility(View.GONE);
            return;
        }
        StringBuilder summary = new StringBuilder();
        summary.append(diagnosticCount).append(diagnosticCount == 1 ? " diagnostic" : " diagnostics");
        if (errors > 0) summary.append(" · ").append(errors).append(errors == 1 ? " error" : " errors");
        if (warnings > 0) summary.append(" · ").append(warnings).append(warnings == 1 ? " warning" : " warnings");
        summary.append(" · tap for details");
        previewError.setText(summary.toString());
        previewError.setVisibility(View.VISIBLE);
        previewError.setClickable(true);
        previewError.setOnClickListener(new View.OnClickListener() {
            @Override public void onClick(View v) { showDiagnostics(); }
        });
    }

    private static String displayDiagnosticSeverity(JSONObject diagnostic) {
        // A free math variable can be valid Typst with no document definition.
        if ("unresolved-name".equals(diagnostic.optString("code"))) return "note";
        return diagnostic.optString("severity", "warning");
    }

    private void clearDiagnostics() {
        currentDiagnostics = new JSONArray();
        diagnosticCount = 0;
        setStatus(statusText);
        previewError.setOnClickListener(null);
        previewError.setClickable(false);
        if (renderedPages.isEmpty()) {
            previewError.setVisibility(View.GONE);
        } else {
            previewError.setText("Preview out of date");
            previewError.setVisibility(View.VISIBLE);
        }
    }

    /** Status error/warning count opens this source-linked list. */
    private void showDiagnostics() {
        if (currentDiagnostics.length() == 0) return;
        ScrollView scroll = new ScrollView(this);
        LinearLayout rows = new LinearLayout(this);
        rows.setOrientation(LinearLayout.VERTICAL);
        rows.setPadding(dp(12), dp(6), dp(12), dp(10));
        scroll.addView(rows);
        final AlertDialog dialog = new AlertDialog.Builder(this, android.R.style.Theme_Material_Dialog_Alert)
                .setTitle("Diagnostics")
                .setView(scroll)
                .setNegativeButton("Close", null)
                .create();
        for (int i = 0; i < currentDiagnostics.length(); i++) {
            final JSONObject diagnostic = currentDiagnostics.optJSONObject(i);
            if (diagnostic == null) continue;
            String severity = displayDiagnosticSeverity(diagnostic);
            String code = diagnostic.optString("code", "diagnostic");
            String provenance = diagnostic.optString("provenance", "document");
            String markerPair = citationKeyText(diagnostic.optJSONArray("markerPair"));
            String label = severity.toUpperCase(java.util.Locale.ROOT) + " · " + code
                    + " · " + provenance + (markerPair.isEmpty() ? "" : " · #" + markerPair.replace(", ", " #"));
            final int start = diagnostic.isNull("start") ? -1 : diagnostic.optInt("start", -1);
            final int end = diagnostic.isNull("end") ? start : diagnostic.optInt("end", start);
            TextView row = panelLabel(label + "\n" + diagnostic.optString("message", ""));
            row.setTextColor(Color.parseColor("error".equals(severity) ? "#F2A7A7" : "#D9C98A"));
            if (start >= 0) {
                row.setClickable(true);
                row.setContentDescription("Open diagnostic source: " + code);
                row.setOnClickListener(new View.OnClickListener() {
                    @Override public void onClick(View v) {
                        dialog.dismiss();
                        navigateTo("diagnostic", "Diagnostic", start, Math.max(start, end), true);
                    }
                });
            }
            rows.addView(row);
        }
        dialog.show();
    }

    /** Applies one worker-decoded chunk to the page stack. */
    private void applyChunk(DecodedChunk chunk) {
        long applyStart = System.nanoTime();
        try {
            if (!chunk.error.isEmpty()) {
                showPreviewError(chunk.error, chunk.diagnostics);
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
            cleanupClaimedTransfers();
            if (!initialPagePending) {
                initialPagePending = true;
                locateInitialPage(editor.getSelectionStart());
            } else if (!chunk.intent.isEmpty()) {
                finishPageIntent(chunk.start, chunk.intent);
            }
            trimPageBitmaps();
            ensureVisiblePages();
            restorePreviewScroll();
            if (searchBar.getVisibility() == View.VISIBLE && findScrollPending) {
                main.post(new Runnable() {
                    @Override public void run() {
                        if (searchBar.getVisibility() == View.VISIBLE && findScrollPending) {
                            showCurrentFindMatch();
                        }
                    }
                });
            }
            setDiagnostics(chunk.diagnostics, chunk.layoutId);
            if (chunk.diagnostics.length() == 0 && chunk.warnings.length() > 0) {
                StringBuilder message = new StringBuilder();
                for (int i = 0; i < chunk.warnings.length(); i++) {
                    if (i > 0) message.append('\n');
                    message.append(chunk.warnings.getString(i));
                }
                previewError.setText(message.toString());
                previewError.setVisibility(View.VISIBLE);
            }
        } catch (Exception e) {
            showPreviewError("render: " + e.getMessage(), null);
        } finally {
            trace("renderCost", "uiApplyMs", (System.nanoTime() - applyStart) / 1_000_000.0,
                    "start", chunk.start, "pages", chunk.pages.size());
        }
    }

    /**
     * Detach scenes before their page views are destroyed. A view zoom or
     * font reflow keeps the same document and the same public terminal
     * reference, so the parser, scrollback and input state can be transplanted
     * into the replacement page. New/Open rotates the document identity and
     * closes them instead.
     */
    private void collectTerminalScenesForRebuild(boolean preserve) {
        if (!preserve) {
            for (TerminalTransfer transfer : pendingTerminalTransfers) {
                transfer.scene.close();
            }
            pendingTerminalTransfers.clear();
        }
        for (RenderedPage page : renderedPages) {
            if (page.terminalScenes != null) {
                for (int i = 0; i < page.terminalScenes.length; i++) {
                    NativeTerminal scene = page.terminalScenes[i];
                    if (scene == null) continue;
                    String reference = i < page.figures.length()
                            ? jsonString(page.figures.optJSONObject(i), "reference") : "";
                    if (preserve && !reference.isEmpty()) {
                        pendingTerminalTransfers.add(new TerminalTransfer(scene,
                                reference, page.index, page.terminalFocus == i,
                                page.terminalWatchOnly != null
                                        && i < page.terminalWatchOnly.length
                                        && page.terminalWatchOnly[i],
                                page.terminalConflict != null
                                        && i < page.terminalConflict.length
                                        && page.terminalConflict[i]));
                    } else {
                        scene.close();
                    }
                    page.terminalScenes[i] = null;
                }
                page.terminalScenes = null;
                page.terminalGridWidths = null;
                page.terminalGridHeights = null;
                page.terminalWatchOnly = null;
                page.terminalConflict = null;
            }
            page.terminalFocus = -1;
        }
        terminalMousePage = null;
        terminalMouseIndex = -1;
        terminalMousePressed = false;
        clearTerminalSelection();
        if (terminalKeyRow != null) terminalKeyRow.setVisibility(View.GONE);
        trace("terminalTransfer", "collect", "pending", pendingTerminalTransfers.size(),
                "preserve", preserve);
    }

    /** Reattach a surviving scene to the rebuilt page's matching reference. */
    private void attachTransferredScenes(RenderedPage page, JSONArray figures) {
        if (pendingTerminalTransfers.isEmpty() || figures == null) return;
        NativeTerminal[] scenes = null;
        int[] widths = null;
        int[] heights = null;
        boolean[] watchOnly = null;
        boolean[] conflict = null;
        int focus = -1;
        for (int i = 0; i < figures.length(); i++) {
            JSONObject figure = figures.optJSONObject(i);
            String reference = jsonString(figure, "reference");
            if (reference.isEmpty()) continue;
            for (TerminalTransfer transfer : pendingTerminalTransfers) {
                if (transfer.claimed || !transfer.reference.equals(reference)) continue;
                if (scenes == null) {
                    scenes = new NativeTerminal[figures.length()];
                    widths = new int[figures.length()];
                    heights = new int[figures.length()];
                    watchOnly = new boolean[figures.length()];
                    conflict = new boolean[figures.length()];
                }
                transfer.claimed = true;
                scenes[i] = transfer.scene;
                watchOnly[i] = transfer.watchOnly;
                conflict[i] = transfer.conflict;
                // Force the first draw after the rebuild to compare the new
                // placed rect instead of treating the scene as already sized.
                widths[i] = 1;
                heights[i] = 1;
                if (transfer.focused) focus = i;
                trace("terminalTransfer", "attach", "page", page.index,
                        "index", i, "reference", reference);
                break;
            }
        }
        if (scenes == null) return;
        page.terminalScenes = scenes;
        page.terminalGridWidths = widths;
        page.terminalGridHeights = heights;
        page.terminalWatchOnly = watchOnly;
        page.terminalConflict = conflict;
        page.terminalFocus = focus;
        page.invalidate();
        final int restoredFocus = focus;
        if (restoredFocus >= 0) {
            if (terminalKeyRow != null) terminalKeyRow.setVisibility(View.VISIBLE);
            main.post(new Runnable() {
                @Override public void run() {
                    if (renderedPages.contains(page) && page.terminalFocus == restoredFocus) {
                        focusTerminal(page, restoredFocus);
                    }
                }
            });
        }
    }

    private void cleanupClaimedTransfers() {
        for (int i = pendingTerminalTransfers.size() - 1; i >= 0; i--) {
            if (pendingTerminalTransfers.get(i).claimed) {
                pendingTerminalTransfers.remove(i);
            }
        }
    }

    /** Create one full-height placeholder per page, then materialize on demand. */
    private void resetPageStack(int total, JSONArray sizes) {
        boolean preserve = session.documentId().equals(pageStackDocumentId);
        collectTerminalScenesForRebuild(preserve);
        pageStackDocumentId = session.documentId();
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
            previewPages.addView(page, new LinearLayout.LayoutParams(
                    width, pageHeight(width, widthPt, heightPt)));
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

    private RenderedPage terminalFocusedPage() {
        for (RenderedPage page : renderedPages) {
            if (page.terminalFocus >= 0) return page;
        }
        return null;
    }

    /** Fixture/attach entry point: start one native scene in a terminal slot. */
    private void startVtScene(RenderedPage page) {
        if (!vtSceneFixturePending || page == null || page.terminalScenes != null) return;
        int index = -1;
        for (int i = 0; i < page.figures.length(); i++) {
            JSONObject figure = page.figures.optJSONObject(i);
            if (figure != null && !jsonString(figure, "reference").isEmpty()) {
                index = i;
                break;
            }
        }
        if (index < 0) return;
        android.graphics.RectF rect = page.terminalRect(index);
        if (rect == null) return;
        int cols = Math.max(20, Math.min(100, (int) (rect.width() / TERMINAL_CELL_WIDTH)));
        int rows = Math.max(5, Math.min(30, (int) (rect.height() / TERMINAL_CELL_HEIGHT)));
        String reference = jsonString(page.figures.optJSONObject(index), "reference");
        // A paired gateway is reached through the registry; the lane's fixture
        // extra is only an override for the same build. No target means the
        // figure stays dormant rather than dialling a guess.
        GatewayTarget gatewayTarget;
        try {
            gatewayTarget = GatewayTarget.resolve(fixtureGateway, fixtureGatewayGeneration,
                    reference, gatewayEndpoints);
            // The device lane cannot resolve a tailnet name, so a row paired
            // through the screen is reached at loopback; the target is still
            // the registry's, and only its origin moves.
            if (gatewayTarget != null) gatewayTarget = gatewayTarget.remap(fixtureConnect);
        } catch (IllegalArgumentException e) {
            trace("gatewayLease", "phase", "failed", "reason", String.valueOf(e.getMessage()),
                    "target", String.valueOf(fixtureGateway),
                    "connect", String.valueOf(fixtureConnect));
            gatewayTarget = null;
        }
        boolean gatewayLeaseBacked = gatewayTarget != null;
        NativeTerminal scene = new NativeTerminal(cols, rows, TERMINAL_CELL_WIDTH,
                TERMINAL_CELL_HEIGHT, 14, !gatewayLeaseBacked);
        if (!scene.valid()) {
            trace("vtScene", "phase", "failed", "reason", "create");
            return;
        }
        page.terminalScenes = new NativeTerminal[page.figures.length()];
        page.terminalScenes[index] = scene;
        page.terminalGridWidths = new int[page.figures.length()];
        page.terminalGridHeights = new int[page.figures.length()];
        page.terminalWatchOnly = new boolean[page.figures.length()];
        page.terminalConflict = new boolean[page.figures.length()];
        SessionCatalog.Entry entry = sessionCatalog.byReference(reference);
        page.terminalWatchOnly[index] = gatewayLeaseBacked || (entry != null
                && entry.state == SessionCatalog.State.WATCHED);
        trace("terminalInput", "control", "watch", page.terminalWatchOnly[index],
                "reason", gatewayLeaseBacked ? "gateway-initial" : "initial");
        page.terminalGridWidths[index] = Math.round(rect.width());
        page.terminalGridHeights[index] = Math.round(rect.height());
        vtSceneFixturePending = false;
        if (gatewayLeaseBacked) {
            // The gateway attach supplies the first DATA after its result. Do
            // not run the synthetic animation over a real lease stream.
            startGatewayLease(gatewayTarget, reference);
        } else {
            byte[] initial = ("\u001b[2J\u001b[Hemthin native VT\r\n"
                    + "frame 0 \u00b7 \u754c e\u0301\r\n$ ").getBytes(java.nio.charset.StandardCharsets.UTF_8);
            JSONObject initialState = scene.feed(initial);
            String initialStateText = initialState.optString("text", "").replace('\n', '|');
            trace("vtScene", "phase", "first", "frame", initialState.optInt("frame", -1),
                    "text", initialStateText.substring(0, Math.min(48, initialStateText.length())));
            vtSceneFixtureRunning = true;
            vtSceneFrame = 0;
        }
        page.invalidate();
        int[] loc = new int[2];
        page.getLocationOnScreen(loc);
        trace("vtScene", "phase", "ready", "page", page.index, "index", index,
                "cols", cols, "rows", rows,
                "x", Math.round(rect.left), "y", Math.round(rect.top),
                "w", Math.round(rect.width()), "h", Math.round(rect.height()),
                "sx", loc[0] + Math.round(rect.left), "sy", loc[1] + Math.round(rect.top));
        revealTerminalTop(page, index);
        if (!gatewayLeaseBacked) {
            main.postDelayed(vtSceneAnimator(), 120);
        } else {
            final RenderedPage placedPage = page;
            final int placedIndex = index;
            main.postDelayed(new Runnable() {
                @Override public void run() { traceScenePlaced(placedPage, placedIndex); }
            }, 150);
        }
    }

    /** Publish the scene's page-local placement once layout has settled. */
    private void traceScenePlaced(RenderedPage page, int index) {
        if (page == null) return;
        android.graphics.RectF placed = page.terminalRect(index);
        int[] loc = new int[2];
        page.getLocationOnScreen(loc);
        if (placed != null) {
            trace("vtScene", "phase", "placed",
                    "x", loc[0] + Math.round(placed.left),
                    "y", loc[1] + Math.round(placed.top),
                    "w", Math.round(placed.width()),
                    "h", Math.round(placed.height()));
        }
    }

    /**
     * Connect the visible scene to its gateway — the paired endpoint for this
     * figure's Core ID, or the lane's scripted fixture when that extra is set.
     * The lease is deliberately transport-owned: Watch/Take request a new
     * attach, and a CLOSE frame revokes the old viewer rather than mutating a
     * flag.
     */
    private void startGatewayLease(final GatewayTarget target, final String reference) {
        gatewayLeaseReference = reference;
        gatewayLeaseRetried = false;
        GatewayTransport.Connector connector;
        if (target.secure) {
            // A paired gateway is a tailnet host behind a public CA, so it uses
            // the platform trust store; only the lane's fixture supplies its own
            // certificate.
            javax.net.ssl.SSLContext context;
            try {
                context = target.fixtureCertificate
                        ? PemTrust.load(fixtureGatewayCa == null ? "" : fixtureGatewayCa.trim())
                        : PemTrust.system();
            } catch (java.io.IOException e) {
                trace("gatewayLease", "phase", "failed", "reason", "ca",
                        "detail", e.getMessage());
                return;
            }
            connector = WssTransport.connector(target.host, target.port,
                    context, 5000, 30000);
        } else {
            connector = GatewayLeaseClient.tcpConnector(target.host, target.port);
        }
        gatewayReady = false;
        gatewayConnected = false;
        gatewayAttaching = false;
        gatewayLease = new GatewayLeaseClient(connector, target.coreId, target.address,
                target.generation,
                new GatewayLeaseClient.Listener() {
                    @Override public void onReady(final String mode, final String leaseId) {
                        main.post(new Runnable() {
                            @Override                            public void run() {
                                gatewayReady = true;
                                gatewayConnected = true;
                                gatewayAttaching = false;
                                RenderedPage page = vtScenePage();
                                int index = vtSceneIndex(page);
                                if (page == null || index < 0) return;
                                boolean control = "control".equals(mode);
                                page.terminalWatchOnly[index] = !control;
                                page.terminalConflict[index] = false;
                                page.invalidate();
                                trace("gatewayLease", "phase", "ready", "mode", mode,
                                        "lease", leaseId == null ? "none" : leaseId,
                                        "page", page.index, "index", index,
                                        "readOnly", !control);
                            }
                        });
                    }

                    @Override public void onClosed(final String reason, final boolean revoked) {
                        main.post(new Runnable() {
                            @Override                            public void run() {
                                gatewayReady = false;
                                gatewayAttaching = false;
                                RenderedPage page = vtScenePage();
                                int index = vtSceneIndex(page);
                                if (page == null || index < 0) return;
                                page.terminalWatchOnly[index] = true;
                                page.terminalConflict[index] = revoked;
                                page.invalidate();
                                trace("gatewayLease", "closed", "reason", reason,
                                        "revoked", revoked, "page", page.index,
                                        "index", index);
                            }
                        });
                    }

                    @Override public void onData(final byte[] bytes) {
                        main.post(new Runnable() {
                            @Override public void run() {
                                RenderedPage page = vtScenePage();
                                int index = vtSceneIndex(page);
                                if (page == null || index < 0) return;
                                page.terminalScenes[index].feed(bytes);
                                page.invalidate();
                                String text = new String(bytes,
                                        java.nio.charset.StandardCharsets.UTF_8)
                                        .replace('\n', '|').replace('\r', ' ');
                                trace("gatewayLease", "data", "bytes", bytes.length,
                                        "text", text.substring(0, Math.min(64, text.length())));
                            }
                        });
                    }

                    @Override public void onResize(final int cols, final int rows) {
                        main.post(new Runnable() {
                            @Override public void run() {
                                RenderedPage page = vtScenePage();
                                int index = vtSceneIndex(page);
                                if (page == null || index < 0) return;
                                page.terminalScenes[index].resize(cols, rows);
                                page.invalidate();
                                trace("gatewayLease", "resize", "cols", cols, "rows", rows);
                            }
                        });
                    }

                    @Override public void onError(final String kind) {
                        main.post(new Runnable() {
                            @Override                            public void run() {
                                gatewayReady = false;
                                // A lease conflict is retryable; a transport or
                                // protocol failure is not.
                                boolean dead = "transport".equals(kind)
                                        || "closed".equals(kind)
                                        || "protocol".equals(kind);
                                // A lease that dies on the wire asks the registry
                                // where its figure's Core ID lives now, and dials
                                // that host once. `gatewayLease == null` means the
                                // lease was closed deliberately, not lost.
                                if (dead && !gatewayLeaseRetried && gatewayLease != null) {
                                    gatewayLeaseRetried = true;
                                    if (retryLeaseOnPairedHost(target, reference)) return;
                                }
                                gatewayConnected = !dead;
                                gatewayReady = gatewayConnected;
                                gatewayAttaching = false;
                                RenderedPage page = vtScenePage();
                                int index = vtSceneIndex(page);
                                if (page != null && index >= 0) {
                                    page.terminalWatchOnly[index] = true;
                                    page.terminalConflict[index] = true;
                                    page.invalidate();
                                }
                                trace("gatewayLease", "error", "kind", kind);
                            }
                        });
                    }
                });
        trace("gatewayLease", "phase", "connect", "host", target.host, "port", target.port,
                "address", target.address,
                "generation", target.generation == null ? "discover" : target.generation,
                "secure", target.secure, "core", target.coreId);
        gatewayLease.connect();
    }

    /**
     * Re-resolve the figure's Core ID through the registry and dial that host,
     * once per lease.
     *
     * @return true when a fresh lease was started, so the caller stops
     */
    private boolean retryLeaseOnPairedHost(GatewayTarget failed, String reference) {
        GatewayTarget next;
        try {
            next = GatewayTarget.retryFor(reference, gatewayEndpoints,
                    failed.host, failed.port);
            if (next != null) next = next.remap(fixtureConnect);
        } catch (IllegalArgumentException e) {
            trace("gatewayLease", "phase", "retry", "host", failed.host,
                    "failed", "malformed");
            return false;
        }
        if (next == null || (next.host.equals(failed.host) && next.port == failed.port)) {
            trace("gatewayLease", "phase", "retry", "host", failed.host,
                    "failed", next == null ? "unpaired" : "same-origin");
            return false;
        }
        trace("gatewayLease", "phase", "retry", "host", next.host, "port", next.port,
                "from", failed.host, "core", next.coreId);
        closeGatewayLease();
        startGatewayLease(next, reference);
        // Starting a lease resets the flag; a retry is still the last one.
        gatewayLeaseRetried = true;
        return true;
    }

    private void closeGatewayLease() {
        gatewayReady = false;
        gatewayConnected = false;
        gatewayAttaching = false;
        GatewayLeaseClient current = gatewayLease;
        gatewayLease = null;
        if (current != null) current.close();
    }

    /** A figure near a page edge can start above the preview viewport. */
    private void revealTerminalTop(RenderedPage page, int index) {
        if (page == null) return;
        page.post(new Runnable() {
            @Override public void run() {
                android.graphics.RectF placed = page.terminalRect(index);
                if (placed == null) return;
                int[] pageLoc = new int[2];
                int[] scrollLoc = new int[2];
                page.getLocationOnScreen(pageLoc);
                previewScroll.getLocationOnScreen(scrollLoc);
                int topInScroll = pageLoc[1] + Math.round(placed.top) - scrollLoc[1];
                int target = Math.max(0,
                        previewScroll.getScrollY() + topInScroll - dp(24));
                trace("vtScene", "reveal", "scroll", target, "top", topInScroll);
                previewScroll.scrollTo(previewScroll.getScrollX(), target);
            }
        });
    }

    /** The current page carrying the fixture scene, which may have moved
     *  across a view-zoom rebuild. */
    private RenderedPage vtScenePage() {
        for (RenderedPage page : renderedPages) {
            if (page.terminalScenes == null) continue;
            for (NativeTerminal scene : page.terminalScenes) {
                if (scene != null) return page;
            }
        }
        return null;
    }

    private int vtSceneIndex(RenderedPage page) {
        if (page == null || page.terminalScenes == null) return -1;
        for (int i = 0; i < page.terminalScenes.length; i++) {
            if (page.terminalScenes[i] != null) return i;
        }
        return -1;
    }

    private Runnable vtSceneAnimator() {
        return new Runnable() {
            @Override public void run() {
                RenderedPage page = vtScenePage();
                int index = vtSceneIndex(page);
                if (!vtSceneFixtureRunning || page == null || index < 0) return;
                NativeTerminal scene = page.terminalScenes[index];
                vtSceneFrame++;
                if (vtSceneFrame == 1) {
                    traceScenePlaced(page, index);
                    revealTerminalTop(page, index);
                }
                if (vtSceneFrame == 4) {
                    scene.recreate();
                    page.recreateTerminalSurface(index);
                    trace("vtScene", "phase", "context-loss", "frame", vtSceneFrame);
                }
                if (vtSceneFrame == 7) {
                    JSONObject before = scene.state();
                    scene.resize(before.optInt("cols", 20) + 2, before.optInt("rows", 5));
                    trace("vtScene", "phase", "resize", "cols", before.optInt("cols", 0) + 2,
                            "rows", before.optInt("rows", 0));
                }
                if (vtSceneFrame == 9) {
                    scene.feed("\u001b[?1000h\u001b[?1006h"
                            .getBytes(java.nio.charset.StandardCharsets.UTF_8));
                    trace("vtScene", "phase", "mouse-mode");
                }
                if (vtSceneA11yFixture && vtSceneFrame == 12) {
                    runVtSceneA11yFixture(page, index);
                }
                if (vtScenePagesFixture && vtSceneFrame == 10 && renderedPages.size() > 1) {
                    RenderedPage next = renderedPages.get(
                            Math.min(page.index + 1, renderedPages.size() - 1));
                    int[] pageLoc = new int[2];
                    int[] scrollLoc = new int[2];
                    next.getLocationOnScreen(pageLoc);
                    previewScroll.getLocationOnScreen(scrollLoc);
                    int topInScroll = pageLoc[1] - scrollLoc[1];
                    int target = Math.max(0,
                            previewScroll.getScrollY() + topInScroll - dp(24));
                    trace("vtScene", "phase", "paging", "away", "page", page.index,
                            "targetPage", next.index, "scroll", target);
                    previewScroll.scrollTo(previewScroll.getScrollX(), target);
                }
                if (vtScenePagesFixture && vtSceneFrame == 14) {
                    int[] pageLoc = new int[2];
                    int[] scrollLoc = new int[2];
                    page.getLocationOnScreen(pageLoc);
                    previewScroll.getLocationOnScreen(scrollLoc);
                    android.graphics.RectF placed = page.terminalRect(index);
                    int topInScroll = pageLoc[1] - scrollLoc[1]
                            + (placed == null ? 0 : Math.round(placed.top));
                    int target = Math.max(0,
                            previewScroll.getScrollY() + topInScroll - dp(24));
                    trace("vtScene", "phase", "paging", "back", "page", page.index,
                            "scroll", target);
                    previewScroll.scrollTo(previewScroll.getScrollX(), target);
                }
                if (vtScenePagesFixture && vtSceneFrame == 16) {
                    RenderedPage current = vtScenePage();
                    int currentIndex = vtSceneIndex(current);
                    if (current != null && currentIndex >= 0) {
                        JSONObject state = current.terminalScenes[currentIndex].state();
                        TerminalSurface cycleSurface = current.terminalSurfaces != null
                                && currentIndex < current.terminalSurfaces.length
                                ? current.terminalSurfaces[currentIndex] : null;
                        trace("vtScene", "phase", "page-cycle", "page", current.index,
                                "index", currentIndex, "loaded", current.loaded,
                                "scroll", previewScroll.getScrollY(),
                                "frame", state.optInt("frame", 0),
                                "generation", state.optInt("generation", 0),
                                "surface", cycleSurface != null,
                                "visibility", cycleSurface == null ? -1
                                        : cycleSurface.getVisibility(),
                                "shown", cycleSurface != null && cycleSurface.isShown());
                        if (cycleSurface != null) cycleSurface.repaint("page-cycle");
                    }
                }
                if (vtSceneZoomFixture && vtSceneFrame == 8) {
                    JSONObject before = scene.state();
                    android.graphics.RectF beforeRect = page.terminalRect(index);
                    trace("vtScene", "phase", "zoom",
                            "cols", before.optInt("cols", 0),
                            "rows", before.optInt("rows", 0),
                            "w", beforeRect == null ? 0 : Math.round(beforeRect.width()),
                            "h", beforeRect == null ? 0 : Math.round(beforeRect.height()));
                    changeViewZoom(0.5f);
                }
                String line = "\u001b[1;1H\u001b[2Kframe " + vtSceneFrame
                        + " \u00b7 \u754c e\u0301";
                scene.feed(line.getBytes(java.nio.charset.StandardCharsets.UTF_8));
                if (vtSceneFrame % 5 == 0) {
                    JSONObject state = scene.state();
                    if (vtSceneFrame == 5) revealTerminalTop(page, index);
                    trace("vtScene", "phase", "frame", vtSceneFrame,
                            "generation", state.optInt("generation", 0),
                            "input", state.optString("input", ""));
                }
                page.invalidate();
                main.postDelayed(this, 120);
            }
        };
    }

    private void focusTerminal(RenderedPage page, int index) {
        if (page == null || page.terminalScenes == null || index < 0
                || index >= page.terminalScenes.length || page.terminalScenes[index] == null) return;
        RenderedPage previous = terminalFocusedPage();
        if (previous != null && previous != page) releaseTerminalFocus();
        page.terminalFocus = index;
        page.requestFocus();
        page.invalidate();
        if (terminalKeyRow != null) terminalKeyRow.setVisibility(View.VISIBLE);
        android.view.inputmethod.InputMethodManager ime =
                (android.view.inputmethod.InputMethodManager) getSystemService(INPUT_METHOD_SERVICE);
        if (ime != null) {
            ime.restartInput(page);
            ime.showSoftInput(page, android.view.inputmethod.InputMethodManager.SHOW_IMPLICIT);
        }
        trace("terminalInput", "phase", "focus", "page", page.index, "index", index);
    }

    private void releaseTerminalFocus() {
        RenderedPage page = terminalFocusedPage();
        if (page == null) return;
        int index = page.terminalFocus;
        page.terminalFocus = -1;
        terminalComposing = "";
        if (terminalKeyRow != null) terminalKeyRow.setVisibility(View.GONE);
        android.view.inputmethod.InputMethodManager ime =
                (android.view.inputmethod.InputMethodManager) getSystemService(INPUT_METHOD_SERVICE);
        if (ime != null) ime.hideSoftInputFromWindow(page.getWindowToken(), 0);
        page.invalidate();
        focusRenderedCaret();
        trace("terminalInput", "phase", "release", "page", page.index, "index", index);
    }

    /** Explicit viewer authority: Watch follows output; Take requests input. */
    private void setTerminalControl(boolean watchOnly) {
        RenderedPage page = terminalFocusedPage();
        if (page == null || page.terminalFocus < 0 || page.terminalWatchOnly == null
                || page.terminalFocus >= page.terminalWatchOnly.length) return;
        if (gatewayLease != null) {
            if (!gatewayConnected || gatewayAttaching) {
                trace("gatewayLease", "phase", "request-pending",
                        watchOnly ? "watch" : "control");
                return;
            }
            // Do not expose input while the new attach is in flight. The
            // gateway result (or an in_use/revoked CLOSE) is authoritative.
            page.terminalWatchOnly[page.terminalFocus] = true;
            gatewayReady = false;
            gatewayAttaching = true;
            trace("gatewayLease", "phase", "request",
                    watchOnly ? "watch" : "control",
                    "page", page.index, "index", page.terminalFocus);
            page.invalidate();
            gatewayLease.switchMode(watchOnly);
            return;
        }
        page.terminalWatchOnly[page.terminalFocus] = watchOnly;
        if (page.terminalConflict != null && page.terminalFocus < page.terminalConflict.length) {
            page.terminalConflict[page.terminalFocus] = false;
        }
        trace("terminalInput", "control", watchOnly ? "watch" : "take",
                "page", page.index, "index", page.terminalFocus);
        page.invalidate();
    }

    private boolean terminalInputBlocked(RenderedPage page, int index, String kind) {
        if (page == null || index < 0) return true;
        boolean watchOnly = page.terminalWatchOnly != null
                && index < page.terminalWatchOnly.length && page.terminalWatchOnly[index];
        boolean conflict = page.terminalConflict != null
                && index < page.terminalConflict.length && page.terminalConflict[index];
        if (!watchOnly && !conflict) return false;
        trace("terminalInput", "blocked", conflict ? "conflict" : "watch",
                "kind", kind, "page", page.index, "index", index);
        return true;
    }

    private void sendTerminalKey(int androidKey, int extraMods, String text, int unshifted) {
        RenderedPage page = terminalFocusedPage();
        if (page == null || page.terminalScenes == null || page.terminalFocus < 0) return;
        if (terminalInputBlocked(page, page.terminalFocus, "key")) return;
        NativeTerminal scene = page.terminalScenes[page.terminalFocus];
        if (scene == null) return;
        int key = NativeTerminal.ghosttyKey(androidKey);
        JSONObject state = scene.key(key, extraMods, text, unshifted);
        if (deviceTest) {
            trace("terminalInput", "key", androidKey, "ghostty", key,
                    "input", lastInput(state));
        }
        sendGatewayInput(state);
        page.invalidate();
    }

    private void sendTerminalText(String text) {
        RenderedPage page = terminalFocusedPage();
        if (page == null || page.terminalFocus < 0) return;
        sendTerminalTextTo(page, page.terminalFocus, text);
    }

    private void sendTerminalTextTo(RenderedPage page, int index, String text) {
        if (page == null || page.terminalScenes == null || index < 0
                || index >= page.terminalScenes.length || text == null || text.isEmpty()) return;
        if (terminalInputBlocked(page, index, "text")) return;
        NativeTerminal scene = page.terminalScenes[index];
        if (scene == null) return;
        JSONObject state = scene.text(text);
        if (deviceTest) trace("terminalInput", "text", text, "input", lastInput(state));
        sendGatewayInput(state);
        page.invalidate();
    }

    /** Route a button event to a program that enabled mouse reporting. */
    private void sendTerminalMouse(RenderedPage page, int index, int x, int y,
            int button, boolean press, int mods) {
        if (page == null || page.terminalScenes == null || index < 0
                || index >= page.terminalScenes.length || page.terminalScenes[index] == null) return;
        if (terminalInputBlocked(page, index, "mouse")) return;
        NativeTerminal scene = page.terminalScenes[index];
        JSONObject state = scene.mouse(x, y, button, press, mods);
        trace("terminalInput", "mouse", "page", page.index, "index", index,
                "x", x, "y", y, "button", button, "press", press,
                "input", lastInput(state));
        sendGatewayInput(state);
        page.invalidate();
    }

    private void sendTerminalMouseMotion(RenderedPage page, int index, int x, int y,
            int button, int mods) {
        if (page == null || page.terminalScenes == null || index < 0
                || index >= page.terminalScenes.length || page.terminalScenes[index] == null) return;
        if (terminalInputBlocked(page, index, "motion")) return;
        NativeTerminal scene = page.terminalScenes[index];
        JSONObject state = scene.mouseMotion(x, y, button, mods);
        trace("terminalInput", "motion", "page", page.index, "index", index,
                "x", x, "y", y, "button", button, "input", lastInput(state));
        sendGatewayInput(state);
        page.invalidate();
    }

    /** Long-press selection starts at a word and extends by dragging cells. */
    private void selectTerminalWord(RenderedPage page, int index, float x, float y) {
        if (page == null || page.terminalScenes == null || index < 0
                || index >= page.terminalScenes.length || page.terminalScenes[index] == null) return;
        int[] cell = page.terminalCellAt(index, x, y);
        if (cell == null) return;
        int[] bounds = page.terminalScenes[index].wordSelection(cell[0], cell[1]);
        if (bounds == null) return;
        terminalSelectionPage = page;
        terminalSelectionIndex = index;
        terminalSelX0 = bounds[0];
        terminalSelY0 = bounds[1];
        terminalSelX1 = bounds[2];
        terminalSelY1 = bounds[3];
        terminalSelecting = true;
        page.invalidate();
        trace("terminalSelection", "start", "page", page.index, "index", index,
                "x0", terminalSelX0, "y0", terminalSelY0,
                "x1", terminalSelX1, "y1", terminalSelY1);
    }

    private void updateTerminalSelectionEnd(RenderedPage page, float x, float y) {
        if (terminalSelectionPage != page) return;
        int[] cell = page.terminalCellAt(terminalSelectionIndex, x, y);
        if (cell == null) return;
        terminalSelX1 = cell[0];
        terminalSelY1 = cell[1];
        page.invalidate();
    }

    private String terminalSelectionText() {
        RenderedPage page = terminalSelectionPage;
        if (page == null || page.terminalScenes == null || terminalSelectionIndex < 0
                || terminalSelectionIndex >= page.terminalScenes.length
                || page.terminalScenes[terminalSelectionIndex] == null) return "";
        return page.terminalScenes[terminalSelectionIndex].selection(
                terminalSelX0, terminalSelY0, terminalSelX1, terminalSelY1);
    }

    private void copyTerminalSelection() {
        String text = terminalSelectionText();
        if (text.isEmpty()) return;
        android.content.ClipboardManager clipboard =
                (android.content.ClipboardManager) getSystemService(CLIPBOARD_SERVICE);
        if (clipboard != null) clipboard.setPrimaryClip(
                android.content.ClipData.newPlainText("Terminal", text));
        trace("terminalSelection", "copy", "chars", text.length());
        clearTerminalSelection();
    }

    private void copyTerminalOutput(RenderedPage page, int index) {
        if (page == null || page.terminalScenes == null || index < 0
                || index >= page.terminalScenes.length || page.terminalScenes[index] == null) return;
        NativeTerminal scene = page.terminalScenes[index];
        String text = scene.selection(0, 0,
                Math.max(0, scene.cols() - 1), Math.max(0, scene.rows() - 1));
        if (text.isEmpty()) return;
        android.content.ClipboardManager clipboard =
                (android.content.ClipboardManager) getSystemService(CLIPBOARD_SERVICE);
        if (clipboard != null) clipboard.setPrimaryClip(
                android.content.ClipData.newPlainText("Terminal", text));
        fileMessage("Copied terminal output");
    }

    private void clearTerminalSelection() {
        RenderedPage page = terminalSelectionPage;
        terminalSelectionPage = null;
        terminalSelectionIndex = -1;
        terminalSelecting = false;
        if (page != null) page.invalidate();
        if (terminalSelectionBar != null) terminalSelectionBar.setVisibility(View.GONE);
    }

    private void showTerminalSelectionBar() {
        if (terminalSelectionBar != null && terminalSelectionPage != null) {
            terminalSelectionBar.setVisibility(View.VISIBLE);
        }
    }

    /** Paste clipboard text into a terminal scene as input, not the document. */
    private void pasteTerminalClipboard() {
        RenderedPage page = terminalSelectionPage;
        int index = terminalSelectionIndex;
        if (page == null || page.terminalScenes == null || index < 0
                || index >= page.terminalScenes.length || page.terminalScenes[index] == null) {
            page = terminalFocusedPage();
            index = page == null ? -1 : page.terminalFocus;
        }
        if (page == null || page.terminalScenes == null || index < 0) return;
        if (terminalInputBlocked(page, index, "paste")) return;
        android.content.ClipboardManager clipboard =
                (android.content.ClipboardManager) getSystemService(CLIPBOARD_SERVICE);
        if (clipboard == null || !clipboard.hasPrimaryClip() || clipboard.getPrimaryClip() == null
                || clipboard.getPrimaryClip().getItemCount() == 0) return;
        CharSequence value = clipboard.getPrimaryClip().getItemAt(0).coerceToText(this);
        if (value != null && value.length() > 0) {
            trace("terminalInput", "paste", "chars", value.length());
            sendTerminalTextTo(page, index, value.toString());
        }
        clearTerminalSelection();
    }

    private static String lastInput(JSONObject state) {
        String input = state == null ? "" : state.optString("input", "");
        int space = input.lastIndexOf(' ');
        return space < 0 ? input : input.substring(space + 1);
    }

    /** Send the latest VT-encoded input token over a live controller lease. */
    private void sendGatewayInput(JSONObject state) {
        GatewayLeaseClient lease = gatewayLease;
        if (lease == null || !lease.isControlling()) return;
        String input = state == null ? "" : state.optString("input", "");
        int space = input.lastIndexOf(' ');
        String token = space < 0 ? input : input.substring(space + 1);
        int colon = token.indexOf(':');
        if (colon < 0) return;
        byte[] bytes = decodeHex(token.substring(colon + 1));
        if (bytes == null || bytes.length == 0) return;
        if (lease.sendInput(bytes)) {
            trace("gatewayLease", "input", "bytes", bytes.length);
        } else {
            trace("gatewayLease", "input", "failed", "bytes", bytes.length);
        }
    }

    private static byte[] decodeHex(String hex) {
        if (hex == null || hex.isEmpty() || (hex.length() & 1) != 0) return null;
        byte[] out = new byte[hex.length() / 2];
        for (int i = 0; i < out.length; i++) {
            int high = Character.digit(hex.charAt(i * 2), 16);
            int low = Character.digit(hex.charAt(i * 2 + 1), 16);
            if (high < 0 || low < 0) return null;
            out[i] = (byte) ((high << 4) | low);
        }
        return out;
    }

    private TextView terminalKeyButton(String label, int androidKey) {
        return terminalKeyButton(label, androidKey, false);
    }

    private TextView terminalKeyButton(final String label, final int androidKey, final boolean ctrl) {
        return shortcut(label, new Runnable() {
            @Override public void run() {
                sendTerminalKey(androidKey, ctrl ? 2 : 0, "", 0);
            }
        });
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
                        clearTerminalSelection();
                        touchSelectionPage = page;
                        touchSelectionAnchor = selectionAnchor;
                        touchDownX = event.getX();
                        touchDownY = event.getY();
                        moved = false;
                        touchSelecting = false;
                        terminalSelecting = false;
                        terminalMousePage = null;
                        terminalMouseIndex = -1;
                        terminalMousePressed = false;
                        touchSelectionHandle = -1;
                        main.removeCallbacks(touchLongPress);
                        if (page.loaded) {
                            int handle = page.selectionHandleAt(event.getX(), event.getY());
                            if (handle >= 0) {
                                int begin = Math.min(selectionAnchor, editor.getSelectionStart());
                                int end = Math.max(selectionAnchor, editor.getSelectionStart());
                                setSelectionAnchor(handle == 0 ? end : begin);
                                touchSelectionHandle = handle;
                                touchSelecting = true;
                                showSelectionBar();
                                return true;
                            }
                        }
                        if (!pagesStale && page.loaded) {
                            int terminal = page.terminalIndexAt(event.getX(), event.getY());
                            NativeTerminal scene = terminal >= 0
                                    ? page.terminalScenes[terminal] : null;
                            if (scene != null && scene.mouseTracking()) {
                                focusTerminal(page, terminal);
                                int[] cell = page.terminalCellAt(terminal,
                                        event.getX(), event.getY());
                                if (cell != null) {
                                    terminalMousePage = page;
                                    terminalMouseIndex = terminal;
                                    terminalMousePressed = true;
                                    sendTerminalMouse(page, terminal, cell[0], cell[1],
                                            0, true, 0);
                                }
                            }
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
                        if (terminalSelecting && !pagesStale) {
                            updateTerminalSelectionEnd(page, event.getX(), event.getY());
                        } else if (terminalMousePressed && terminalMousePage == page
                                && !pagesStale) {
                            int[] cell = page.terminalCellAt(terminalMouseIndex,
                                    event.getX(), event.getY());
                            if (cell != null && page.terminalScenes[terminalMouseIndex]
                                    .mouseMotionTracking()) {
                                sendTerminalMouseMotion(page, terminalMouseIndex,
                                        cell[0], cell[1], 0, 0);
                            }
                        } else if (touchSelecting && !pagesStale) {
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
                        if (terminalSelecting) {
                            updateTerminalSelectionEnd(page, event.getX(), event.getY());
                            terminalSelecting = false;
                            showTerminalSelectionBar();
                            touchSelectionPage = null;
                            return true;
                        }
                        if (terminalMousePressed && terminalMousePage == page) {
                            int[] cell = page.terminalCellAt(terminalMouseIndex,
                                    event.getX(), event.getY());
                            if (cell != null) {
                                sendTerminalMouse(page, terminalMouseIndex, cell[0], cell[1],
                                        0, false, 0);
                            }
                            terminalMousePressed = false;
                            terminalMousePage = null;
                            terminalMouseIndex = -1;
                            moved = true;
                        }
                        if (touchSelecting) {
                            int offset = caretOffsetAtPoint(page, event.getX(), event.getY());
                            if (offset >= 0) setDocumentCaret(offset);
                            showSelectionBar();
                        } else if (!moved && !pagesStale) {
                            int terminal = page.terminalIndexAt(event.getX(), event.getY());
                            if (terminal >= 0) {
                                focusTerminal(page, terminal);
                            } else {
                                if (page.terminalFocus >= 0) releaseTerminalFocus();
                                selectPageGlyph(page, event.getX(), event.getY());
                            }
                        }
                        touchSelecting = false;
                        touchSelectionHandle = -1;
                        touchSelectionPage = null;
                        return true;
                    case MotionEvent.ACTION_CANCEL:
                        main.removeCallbacks(touchLongPress);
                        if (terminalMousePressed && terminalMousePage == page) {
                            int[] cell = page.terminalCellAt(terminalMouseIndex,
                                    event.getX(), event.getY());
                            if (cell != null) {
                                sendTerminalMouse(page, terminalMouseIndex, cell[0], cell[1],
                                        0, false, 0);
                            }
                        }
                        terminalMousePressed = false;
                        terminalMousePage = null;
                        terminalMouseIndex = -1;
                        terminalSelecting = false;
                        if (!touchSelecting) {
                            touchSelectionPage = null;
                        }
                        touchSelecting = false;
                        touchSelectionHandle = -1;
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
        Bitmap old = page.raster;
        if (old != null && old != p.bitmap && !old.isRecycled()) {
            retainedBitmapBytes = Math.max(0, retainedBitmapBytes - page.bitmapBytes);
            old.recycle();
        }
        page.raster = p.bitmap;
        page.invalidate();
        page.hits = p.hits;
        page.textTop = p.textTop;
        page.widthPt = p.widthPt;
        page.heightPt = p.heightPt;
        page.hasFigures = p.hasFigures;
        page.figures = p.figures;
        attachTransferredScenes(page, p.figures);
        int terminalFigures = 0;
        for (int i = 0; i < p.figures.length(); i++) {
            JSONObject figure = p.figures.optJSONObject(i);
            if (figure != null && !jsonString(figure, "reference").isEmpty()) terminalFigures++;
        }
        trace("figureMetadata", "page", p.index, "figures", p.figures.length(),
                "terminal", terminalFigures);
        page.bitmapBytes = p.bitmap.getAllocationByteCount();
        if (!page.loaded) {
            page.loaded = true;
            pagesLoaded++;
        }
        if (figureA11yFixturePending && p.figures.length() > 0) {
            runFigureA11yFixture(page);
        }
        retainedBitmapBytes += page.bitmapBytes;
        if (!retainedPages.contains(page)) retainedPages.add(page);
        int width = Math.round(basePageWidth() * viewZoom);
        int height = pageHeight(width, page.widthPt, page.heightPt);
        page.setLayoutParams(new LinearLayout.LayoutParams(
                width, height));
        if (page.caption != null) page.caption.setText("page " + (p.index + 1) + " / " + totalPages);
        trace("pageRaster", "page", p.index, "wh", p.width + "x" + p.height,
                "visiblePx~", p.visiblePixels, "bytes", page.bitmapBytes);
        trace("pageLayout", "page", p.index, "raster", p.width + "x" + p.height,
                "pw", width, "ih", height, "textTop", p.textTop);
        if (vtSceneFixturePending) {
            page.post(new Runnable() {
                @Override public void run() { startVtScene(page); }
            });
        }
        if (!inspectorOpen && !(getCurrentFocus() instanceof EditText)) focusRenderedCaret();
    }

    private static int pageHeight(int width, double widthPt, double heightPt) {
        return Math.max(1, (int) Math.round(width * heightPt / Math.max(1.0, widthPt)));
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
            // A background page restore can finish after the source or
            // search field has acquired keyboard focus.
            if (!(getCurrentFocus() instanceof EditText)) focusRenderedCaret();
        } else if ("navigation".equals(intent)) {
            CaretSpot spot = caretSpot(editor.getSelectionStart());
            if (spot.page != null && spot.distance <= 1) {
                focusRenderedCaret();
            } else {
                // A figure-only or hidden destination keeps a source fallback.
                openSourceAt(editor.getSelectionStart(), editor.getSelectionEnd());
            }
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

    private boolean dropPageRaster(RenderedPage page) {
        Bitmap raster = page.raster;
        page.raster = null;
        page.invalidate();
        if (raster == null || raster.isRecycled()) return false;
        raster.recycle();
        return true;
    }

    private void releasePageBitmap(RenderedPage page) {
        dropPageRaster(page);
        if (page.terminalFocus >= 0) {
            page.terminalFocus = -1;
            if (terminalKeyRow != null) terminalKeyRow.setVisibility(View.GONE);
        }
        page.closeTerminalScenes();
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
        for (int i = 0; i < renderedPages.size(); i++) {
            renderedPages.get(i).closeTerminalScenes();
        }
        if (terminalKeyRow != null) terminalKeyRow.setVisibility(View.GONE);
        renderedPages.clear();
        retainedPages.clear();
        retainedBitmapBytes = 0;
        pagesLoaded = 0;
        int released = 0;
        for (int i = previewPages.getChildCount() - 1; i >= 0; i--) {
            View child = previewPages.getChildAt(i);
            if (!(child instanceof RenderedPage)) {
                continue;
            }
            RenderedPage page = (RenderedPage) child;
            if (dropPageRaster(page)) released++;
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
    private void applyUndo() {
        int start = replaceUndoStart, end = replaceUndoEnd;
        replaceUndoStart = replaceUndoEnd = -1;
        applyState(DocBridge.undo(handle), true);
        if (start >= 0 && editor != null) {
            int a = Math.min(start, editor.length());
            int b = Math.min(end, editor.length());
            editor.setSelection(a, b);
            if (a < b) setSelectionAnchor(a); else clearSelectionAnchor();
            invalidateRenderedCaret();
        }
    }

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
            if (!setCaret) replaceUndoStart = replaceUndoEnd = -1;

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
            canRedo = o.getBoolean("canRedo");
            documentMarkers = o.optJSONArray("markers");
            if (documentMarkers == null) documentMarkers = new JSONArray();
            documentModifierLinks = o.optJSONArray("modifierLinks");
            if (documentModifierLinks == null) documentModifierLinks = new JSONArray();
            documentCitations = new JSONArray(DocBridge.citationIndex(text, false));
            refreshSemantics(text);
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
            setStatus("state error: " + e.getMessage());
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
        trace("status", "revision", o.getLong("revision"), "segments", segs.length(),
                "props", props.toString(), "more", segs.length() > shown);
        setStatus("Ready");
    }

    /** Keeps the status line's diagnostic count clickable across status updates. */
    private void setStatus(String text) {
        statusText = text == null ? "" : text;
        if (status == null) return;
        String name = StatusModel.documentName(documentLabel != null ? documentLabel
                : documentUri == null ? null : documentUri.getLastPathSegment());
        String state = StatusModel.saveState(recoveryReady, unsaved,
                activeFile == null ? null : activeFile.kind,
                activeFile != null && activeFile.copy);
        String detail = statusText;
        if (findVisible() && find != null && !find.status().isEmpty()) {
            detail = detail.isEmpty() ? find.status() : detail + " · " + find.status();
        }
        status.setText(StatusModel.compose(name, state, detail, diagnosticCount));
        status.setClickable(diagnosticCount > 0);
        status.setOnClickListener(diagnosticCount > 0 ? new View.OnClickListener() {
            @Override public void onClick(View v) { showDiagnostics(); }
        } : null);
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
