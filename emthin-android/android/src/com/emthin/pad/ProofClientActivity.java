package com.emthin.pad;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.content.pm.PackageManager;
import android.database.Cursor;
import android.net.Uri;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.provider.OpenableColumns;
import android.text.InputType;
import android.util.Base64;
import android.util.Log;
import android.widget.Button;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;

import java.io.InputStream;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/**
 * Native MM0 proof client: pick a {@code .mm0}/{@code .mm1} pair, ask the
 * configured mm0web backend to verify it, publish it on an explicit and
 * confirmed action, follow the backend's publication to its DOI, and hand the
 * proof ID and link back to the note.
 *
 * <p>Verification and Zenodo publication stay in the backend; this Activity
 * holds no credential. Every answer is classified by {@link ProofProtocol}, so
 * a rejected pair is reported as rejected and a refused publication as refused.
 * Results are bound to the captured file pair, server and Activity generation;
 * a late answer for an older pair or a destroyed Activity is dropped. The pair,
 * the pending proof ID and its last known state survive recreation, and a
 * queued publication is re-read on resume.
 *
 * <p>Launch extras for scripted fixtures are honoured only by the disposable
 * test package (its manifest carries {@code com.emthin.pad.deviceTest}).
 */
public final class ProofClientActivity extends Activity {
    private static final String TAG = "emthin-pad";
    private static final int PICK_MM0 = 1;
    private static final int PICK_MM1 = 2;
    private static final String PREFS = "proof-client";
    private static final String KEY_BASE = "base-url";
    private static final String KEY_PROJECT = "project";
    private static final String KEY_MM0 = "mm0-uri";
    private static final String KEY_MM1 = "mm1-uri";
    private static final String KEY_MM0_NAME = "mm0-name";
    private static final String KEY_MM1_NAME = "mm1-name";
    private static final String KEY_PROOF_ID = "proof-id";
    private static final String KEY_STATE = "state";
    private static final String KEY_DOI = "doi";
    private static final String KEY_URL = "url";
    private static final String KEY_MESSAGE = "message";
    private static final String KEY_FIXTURE_SPEC = "fixture-spec";
    private static final String KEY_FIXTURE_PROOF = "fixture-proof";
    private static final String DEFAULT_PROJECT = "unfer";
    private static final String TEST_DEFAULT_BASE = "http://127.0.0.1:18919";
    private static final int CONNECT_TIMEOUT_MS = 10_000;
    /** Two checker stages at the backend's default 60 s each, plus slack. */
    private static final int READ_TIMEOUT_MS = 150_000;
    private static final long POLL_MS = 1500;
    private static final int POLL_LIMIT = 80;

    private final Handler main = new Handler(Looper.getMainLooper());
    private final ExecutorService worker = Executors.newSingleThreadExecutor();
    /** Bumped whenever the pair, server or Activity lifetime changes. */
    private int generation;
    private boolean deviceTest;
    private SharedPreferences prefs;

    private Uri mm0;
    private Uri mm1;
    private String mm0Name = "";
    private String mm1Name = "";
    /** Scripted fixture text for the device lane, bypassing the picker. */
    private String fixtureSpec;
    private String fixtureProof;
    private String proofId = "";
    private String state = "";
    private String doi = "";
    private String url = "";
    private String message = "no proof yet";
    private String sourceDocId = "";
    private boolean busy;
    private int polls;
    private boolean resumed;

    private TextView stateView;
    private EditText serverField;
    private EditText projectField;
    private Button verifyButton;
    private Button publishButton;
    private Button refreshButton;
    private Button insertButton;
    private Button shareButton;
    private LinearLayout layout;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        prefs = getSharedPreferences(PREFS, MODE_PRIVATE);
        deviceTest = deviceTestEnabled();
        proofId = prefs.getString(KEY_PROOF_ID, "");
        state = prefs.getString(KEY_STATE, "");
        doi = prefs.getString(KEY_DOI, "");
        url = prefs.getString(KEY_URL, "");
        message = prefs.getString(KEY_MESSAGE, "no proof yet");
        mm0 = parseUri(prefs.getString(KEY_MM0, null));
        mm1 = parseUri(prefs.getString(KEY_MM1, null));
        mm0Name = prefs.getString(KEY_MM0_NAME, "");
        mm1Name = prefs.getString(KEY_MM1_NAME, "");
        String passedDoc = savedInstanceState != null
                ? savedInstanceState.getString("source-doc-id")
                : getIntent().getStringExtra("emthin.proof_doc_id");
        sourceDocId = passedDoc == null ? "" : passedDoc;
        if (deviceTest) readFixtureExtras(getIntent());

        layout = new LinearLayout(this);
        layout.setOrientation(LinearLayout.VERTICAL);
        int pad = Math.round(16 * getResources().getDisplayMetrics().density);
        layout.setPadding(pad, pad, pad, pad);
        stateView = new TextView(this);
        stateView.setTextSize(15f);
        stateView.setPadding(0, 0, 0, pad);
        stateView.setTextIsSelectable(true);
        stateView.setContentDescription(null);
        layout.addView(stateView);

        serverField = field("Proof server (https://…)", baseUrlSetting(),
                InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_URI);
        projectField = field("Project", prefs.getString(KEY_PROJECT, DEFAULT_PROJECT),
                InputType.TYPE_CLASS_TEXT);
        Button saveServer = button("Save server");
        saveServer.setOnClickListener(v -> saveServer());

        Button pick0 = button("Pick .mm0");
        pick0.setOnClickListener(v -> pick(PICK_MM0));
        Button pick1 = button("Pick .mm1");
        pick1.setOnClickListener(v -> pick(PICK_MM1));
        verifyButton = button("Verify");
        verifyButton.setOnClickListener(v -> submit(false));
        publishButton = button("Publish to Zenodo");
        publishButton.setOnClickListener(v -> confirmPublish());
        refreshButton = button("Refresh status");
        refreshButton.setOnClickListener(v -> refresh(true));
        insertButton = button("Insert into note");
        insertButton.setOnClickListener(v -> insertIntoNote());
        shareButton = button("Share ID / link");
        shareButton.setOnClickListener(v -> shareResult());

        ScrollView scroll = new ScrollView(this);
        scroll.addView(layout);
        setContentView(scroll);
        trace("proof phase open", "test", deviceTest, "id", proofId.isEmpty() ? "-" : proofId,
                "state", state.isEmpty() ? "-" : state);
    }

    private EditText field(String hint, String value, int type) {
        EditText e = new EditText(this);
        e.setHint(hint);
        e.setContentDescription(hint);
        e.setSingleLine(true);
        e.setInputType(type);
        e.setText(value);
        layout.addView(e);
        return e;
    }

    private Button button(String label) {
        Button b = new Button(this);
        b.setText(label);
        b.setContentDescription(label);
        layout.addView(b);
        return b;
    }

    @Override
    protected void onResume() {
        super.onResume();
        resumed = true;
        render();
        // A publication queued before a rotation, backgrounding or relaunch is
        // read again rather than shown as it was left.
        if (!proofId.isEmpty() && ProofProtocol.STATE_QUEUED.equals(state)) {
            polls = 0;
            refresh(false);
        }
    }

    @Override
    protected void onPause() {
        resumed = false;
        // Only the poll timer stops; a request's answer still lands.
        main.removeCallbacks(pollTick);
        super.onPause();
    }

    @Override
    protected void onSaveInstanceState(Bundle outState) {
        super.onSaveInstanceState(outState);
        if (!sourceDocId.isEmpty()) outState.putString("source-doc-id", sourceDocId);
    }

    @Override
    protected void onDestroy() {
        generation++;
        main.removeCallbacksAndMessages(null);
        worker.shutdownNow();
        super.onDestroy();
    }

    private void render() {
        StringBuilder b = new StringBuilder();
        b.append("status: ").append(message);
        b.append("\nproof id: ").append(proofId.isEmpty() ? "(none)" : proofId);
        if (!state.isEmpty()) b.append("\nrecord: ").append(ProofProtocol.describeState(state, doi));
        if (!proofId.isEmpty()) b.append("\nlink: ").append(currentLink());
        b.append("\n.mm0: ").append(fileLabel(fixtureSpec, mm0, mm0Name));
        b.append("\n.mm1: ").append(fileLabel(fixtureProof, mm1, mm1Name));
        stateView.setText(b.toString());
        boolean havePair = fixtureSpec != null || (mm0 != null && mm1 != null);
        verifyButton.setEnabled(!busy && havePair);
        publishButton.setEnabled(!busy && havePair);
        refreshButton.setEnabled(!busy && !proofId.isEmpty());
        insertButton.setEnabled(!proofId.isEmpty());
        shareButton.setEnabled(!proofId.isEmpty());
    }

    private static String fileLabel(String fixture, Uri uri, String name) {
        if (fixture != null) return "(fixture)";
        if (uri == null) return "(not picked)";
        return name.isEmpty() ? String.valueOf(uri.getLastPathSegment()) : name;
    }

    // ---- configuration ---------------------------------------------------

    private String baseUrlSetting() {
        return prefs.getString(KEY_BASE, deviceTest ? TEST_DEFAULT_BASE : "");
    }

    private String[] syncServerInputs() {
        String base = ProofProtocol.checkBase(serverField.getText().toString(), deviceTest);
        String project = ProofProtocol.checkProject(projectField.getText().toString());
        boolean changed = !base.equals(baseUrlSetting())
                || !project.equals(prefs.getString(KEY_PROJECT, DEFAULT_PROJECT));
        prefs.edit().putString(KEY_BASE, base).putString(KEY_PROJECT, project).apply();
        serverField.setText(base);
        projectField.setText(project);
        if (changed) {
            // A record belongs to the server and project that issued it.
            main.removeCallbacks(pollTick);
            generation++;
            clearRecord("server saved");
        }
        trace("proof phase server", "changed", changed);
        return new String[] {base, project};
    }

    private void saveServer() {
        try {
            String hadId = proofId;
            syncServerInputs();
            if (hadId.equals(proofId)) message = "server saved";
        } catch (IllegalArgumentException e) {
            message = e.getMessage();
        }
        render();
    }

    private void readFixtureExtras(Intent intent) {
        String base = intent.getStringExtra("emthin.proof_base");
        String validBase = null;
        if (base != null && !base.trim().isEmpty()) {
            try {
                validBase = ProofProtocol.checkBase(base, true);
                prefs.edit().putString(KEY_BASE, validBase).apply();
            } catch (IllegalArgumentException e) {
                Log.w(TAG, "proof fixture base refused: " + e.getMessage());
            }
        }
        if (intent.getBooleanExtra("emthin.proof_reset", false)) {
            main.removeCallbacks(pollTick);
            prefs.edit().clear().apply();
            proofId = state = doi = url = "";
            mm0 = mm1 = null;
            mm0Name = mm1Name = "";
            message = "no proof yet";
            if (validBase != null) {
                prefs.edit().putString(KEY_BASE, validBase).apply();
            }
        }
        String spec = decodeExtra(intent, "emthin.proof_spec");
        String proof = decodeExtra(intent, "emthin.proof_text");
        // Persist a scripted fixture so a later toolbar launch (which carries
        // no extras) reuses it for the lane's deterministic run.
        if (spec != null) {
            prefs.edit().putString(KEY_FIXTURE_SPEC, spec)
                    .putString(KEY_FIXTURE_PROOF, proof == null ? spec : proof).apply();
        }
        fixtureSpec = prefs.getString(KEY_FIXTURE_SPEC, null);
        fixtureProof = prefs.getString(KEY_FIXTURE_PROOF, null);
        if (spec != null) {
            main.removeCallbacks(pollTick);
            generation++;
            clearRecord("fixture pair loaded");
        }
    }

    private static String decodeExtra(Intent intent, String key) {
        String b64 = intent.getStringExtra(key + "_b64");
        if (b64 != null) {
            try {
                return new String(Base64.decode(b64, Base64.DEFAULT), StandardCharsets.UTF_8);
            } catch (IllegalArgumentException e) {
                return null;
            }
        }
        return intent.getStringExtra(key);
    }

    private boolean deviceTestEnabled() {
        try {
            Bundle metadata = getPackageManager().getApplicationInfo(getPackageName(),
                    PackageManager.GET_META_DATA).metaData;
            return metadata != null && metadata.getBoolean("com.emthin.pad.deviceTest");
        } catch (PackageManager.NameNotFoundException e) {
            return false;
        }
    }

    // ---- file pair -------------------------------------------------------

    private void pick(int request) {
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
        intent.setType("*/*");
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        intent.addFlags(Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION
                | Intent.FLAG_GRANT_READ_URI_PERMISSION);
        try {
            startActivityForResult(intent, request);
        } catch (android.content.ActivityNotFoundException e) {
            message = "no document picker is available on this device";
            render();
        }
    }

    @Override
    protected void onActivityResult(int request, int code, Intent data) {
        super.onActivityResult(request, code, data);
        if (code != RESULT_OK || data == null || data.getData() == null) return;
        Uri uri = data.getData();
        try {
            getContentResolver().takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION);
        } catch (SecurityException e) {
            Log.w(TAG, "proof file access is temporary", e);
        }
        String name = displayName(uri);
        SharedPreferences.Editor edit = prefs.edit();
        if (request == PICK_MM0) {
            mm0 = uri;
            mm0Name = name;
            edit.putString(KEY_MM0, uri.toString()).putString(KEY_MM0_NAME, name);
        } else if (request == PICK_MM1) {
            mm1 = uri;
            mm1Name = name;
            edit.putString(KEY_MM1, uri.toString()).putString(KEY_MM1_NAME, name);
        } else {
            return;
        }
        // A picked file replaces any scripted pair, and a new pair owns no
        // earlier request's answer or prior proof record.
        fixtureSpec = fixtureProof = null;
        edit.remove(KEY_FIXTURE_SPEC).remove(KEY_FIXTURE_PROOF).apply();
        main.removeCallbacks(pollTick);
        generation++;
        clearRecord("pair changed");
        render();
    }

    private String displayName(Uri uri) {
        try (Cursor c = getContentResolver().query(uri,
                new String[] {OpenableColumns.DISPLAY_NAME}, null, null, null)) {
            if (c != null && c.moveToFirst()) {
                String name = c.getString(0);
                if (name != null) return name;
            }
        } catch (RuntimeException e) {
            Log.w(TAG, "proof file has no display name", e);
        }
        String last = uri.getLastPathSegment();
        return last == null ? "" : last;
    }

    private static Uri parseUri(String s) {
        return s == null || s.isEmpty() ? null : Uri.parse(s);
    }

    private String readText(Uri uri) throws Exception {
        try (InputStream in = getContentResolver().openInputStream(uri)) {
            if (in == null) throw new IllegalStateException("cannot open " + uri.getLastPathSegment());
            byte[] bytes = ProviderIo.readBounded(in, ProofProtocol.MAX_FILE_BYTES);
            return ProviderIo.decodeUtf8(bytes);
        } catch (java.nio.charset.CharacterCodingException e) {
            throw new IllegalStateException("a proof file is not UTF-8 text");
        } catch (java.io.IOException e) {
            String m = e.getMessage();
            throw new IllegalStateException(m != null && m.contains("exceeds")
                    ? "a proof file is larger than 4 MiB" : "cannot read a proof file: " + m);
        } catch (SecurityException e) {
            throw new IllegalStateException("access to a proof file was revoked; pick it again");
        }
    }

    // ---- requests --------------------------------------------------------

    private void confirmPublish() {
        final String base;
        try {
            base = syncServerInputs()[0];
        } catch (IllegalArgumentException e) {
            message = e.getMessage();
            render();
            return;
        }
        String host = Uri.parse(base).getHost();
        new AlertDialog.Builder(this)
                .setTitle("Publish to Zenodo?")
                .setMessage("The server at " + host + " verifies the pair again and, if it is"
                        + " accepted, deposits it on Zenodo. A published record and its DOI"
                        + " are permanent.")
                .setPositiveButton("Publish", (d, w) -> submit(true))
                .setNegativeButton("Cancel", (d, w) -> trace("proof phase publish-cancelled"))
                .show();
    }

    private void submit(final boolean publish) {
        final String base;
        final String project;
        try {
            String[] cfg = syncServerInputs();
            base = cfg[0];
            project = cfg[1];
        } catch (IllegalArgumentException e) {
            message = e.getMessage();
            render();
            return;
        }
        if (fixtureSpec == null && (mm0 == null || mm1 == null)) {
            message = "pick both files first";
            render();
            return;
        }
        final int op = ++generation;
        final Uri pair0 = mm0;
        final Uri pair1 = mm1;
        final String spec = fixtureSpec;
        final String proof = fixtureProof;
        final String title = ProofProtocol.titleFor(spec != null ? "fixture" : mm0Name);
        busy = true;
        message = publish ? "publishing…" : "verifying…";
        render();
        trace("proof phase " + (publish ? "publish" : "verify"), "op", op);
        worker.execute(() -> {
            ProofProtocol.Outcome outcome;
            try {
                String specText = spec != null ? spec : readText(pair0);
                String proofText = spec != null ? proof : readText(pair1);
                Http r = post(base + (publish ? "/api/v1/publish" : "/api/v1/verify"),
                        ProofProtocol.formBody(project, title, specText, proofText));
                outcome = ProofProtocol.parseSubmit(r.code, r.body, publish);
            } catch (Exception e) {
                outcome = ProofProtocol.parseSubmit(-1, e.getMessage(), publish);
            }
            final ProofProtocol.Outcome result = outcome;
            main.post(() -> applyOutcome(op, result));
        });
    }

    private void applyOutcome(int op, ProofProtocol.Outcome outcome) {
        // A result from an older pair, server or a destroyed Activity cannot
        // overwrite the current view.
        if (op != generation || isFinishing() || isDestroyed()) {
            trace("proof phase stale", "op", op, "kind", outcome.kind);
            return;
        }
        busy = false;
        message = outcome.describe();
        if (!outcome.proofId.isEmpty() && ProofProtocol.validId(outcome.proofId)) {
            proofId = outcome.proofId;
            state = outcome.state();
            doi = outcome.doi;
            url = outcome.url;
        }
        persist();
        trace("proof phase result", "kind", outcome.kind, "verified", outcome.verified(),
                "stage", outcome.stage.isEmpty() ? "-" : outcome.stage,
                "id", proofId.isEmpty() ? "-" : proofId, "state", state.isEmpty() ? "-" : state);
        render();
        if (outcome.kind == ProofProtocol.Outcome.Kind.QUEUED) {
            polls = 0;
            schedulePoll();
        }
    }

    private void schedulePoll() {
        if (!resumed || polls >= POLL_LIMIT) {
            if (polls >= POLL_LIMIT) {
                message = "still publishing; tap Refresh status later";
                render();
            }
            return;
        }
        main.removeCallbacks(pollTick);
        main.postDelayed(pollTick, POLL_MS);
    }

    private final Runnable pollTick = () -> refresh(false);

    private void refresh(final boolean manual) {
        if (proofId.isEmpty()) return;
        final String base;
        final String project;
        try {
            base = ProofProtocol.checkBase(baseUrlSetting(), deviceTest);
            project = ProofProtocol.checkProject(prefs.getString(KEY_PROJECT, DEFAULT_PROJECT));
        } catch (IllegalArgumentException e) {
            message = e.getMessage();
            render();
            return;
        }
        final int op = generation;
        final String id = proofId;
        polls++;
        if (manual) {
            busy = true;
            render();
        }
        worker.execute(() -> {
            ProofProtocol.Record record = null;
            String error = null;
            try {
                Http r = get(base + ProofProtocol.proofPath(project, id));
                record = ProofProtocol.parseProof(r.code, r.body, id);
            } catch (IllegalArgumentException e) {
                error = e.getMessage();
            } catch (Exception e) {
                error = "cannot reach the server: " + e.getMessage();
            }
            final ProofProtocol.Record rec = record;
            final String err = error;
            main.post(() -> applyRecord(op, id, rec, err, manual));
        });
    }

    private void applyRecord(int op, String id, ProofProtocol.Record record, String error, boolean manual) {
        if (op != generation || !id.equals(proofId) || isFinishing() || isDestroyed()) return;
        if (manual) busy = false;
        if (record == null) {
            message = error;
            trace("proof phase status-error", "id", id);
            render();
            // A transient failure while queued keeps following the record.
            if (ProofProtocol.STATE_QUEUED.equals(state)) schedulePoll();
            return;
        }
        state = record.state;
        doi = record.doi;
        url = record.url;
        message = ProofProtocol.describeState(state, doi);
        persist();
        trace("proof phase status", "id", id, "state", state, "doi", doi.isEmpty() ? "-" : doi);
        render();
        if (!record.settled()) schedulePoll();
    }

    private void persist() {
        prefs.edit().putString(KEY_PROOF_ID, proofId).putString(KEY_STATE, state)
                .putString(KEY_DOI, doi).putString(KEY_URL, url)
                .putString(KEY_MESSAGE, message).apply();
    }

    private void clearRecord(String why) {
        proofId = state = doi = url = "";
        message = why;
        busy = false;
        persist();
    }

    private static final class Http {
        final int code;
        final String body;

        Http(int code, String body) {
            this.code = code;
            this.body = body;
        }
    }

    private static Http post(String target, String form) throws Exception {
        byte[] bytes = form.getBytes(StandardCharsets.UTF_8);
        HttpURLConnection conn = open(target);
        try {
            conn.setRequestMethod("POST");
            conn.setDoOutput(true);
            conn.setRequestProperty("Content-Type", "application/x-www-form-urlencoded");
            conn.setFixedLengthStreamingMode(bytes.length);
            try (OutputStream out = conn.getOutputStream()) {
                out.write(bytes);
            }
            return response(conn);
        } finally {
            conn.disconnect();
        }
    }

    private static Http get(String target) throws Exception {
        HttpURLConnection conn = open(target);
        try {
            conn.setRequestMethod("GET");
            return response(conn);
        } finally {
            conn.disconnect();
        }
    }

    private static HttpURLConnection open(String target) throws Exception {
        HttpURLConnection conn = (HttpURLConnection) new URL(target).openConnection();
        conn.setConnectTimeout(CONNECT_TIMEOUT_MS);
        conn.setReadTimeout(READ_TIMEOUT_MS);
        // A redirect could move a submission to another origin; the backend
        // never answers these routes with one.
        conn.setInstanceFollowRedirects(false);
        conn.setUseCaches(false);
        conn.setRequestProperty("Accept", "application/json");
        return conn;
    }

    private static Http response(HttpURLConnection conn) throws Exception {
        int code = conn.getResponseCode();
        InputStream in = code < 400 ? conn.getInputStream() : conn.getErrorStream();
        if (in == null) return new Http(code, "");
        try (InputStream body = in) {
            byte[] bytes = ProviderIo.readBounded(body, ProofProtocol.MAX_RESPONSE_CHARS);
            return new Http(code, new String(bytes, StandardCharsets.UTF_8));
        }
    }

    // ---- hand-off --------------------------------------------------------

    private String currentLink() {
        String base;
        try {
            base = ProofProtocol.checkBase(baseUrlSetting(), deviceTest);
        } catch (IllegalArgumentException e) {
            base = "";
        }
        return ProofProtocol.resultLink(base, prefs.getString(KEY_PROJECT, DEFAULT_PROJECT),
                proofId, doi, url);
    }

    /** Return the proof ID and link to MainActivity for insertion into the note. */
    private void insertIntoNote() {
        if (proofId.isEmpty()) {
            message = "no proof id yet";
            render();
            return;
        }
        Intent result = new Intent();
        result.putExtra("emthin.proof_id", proofId);
        result.putExtra("emthin.proof_link", currentLink());
        result.putExtra("emthin.proof_state", state);
        result.putExtra("emthin.proof_doi", doi);
        if (!sourceDocId.isEmpty()) result.putExtra("emthin.proof_doc_id", sourceDocId);
        setResult(RESULT_OK, result);
        trace("proof phase insert", "id", proofId, "state", state.isEmpty() ? "-" : state);
        finish();
    }

    private void shareResult() {
        if (proofId.isEmpty()) {
            message = "no proof id yet";
            render();
            return;
        }
        String link = currentLink();
        ClipboardManager clip = (ClipboardManager) getSystemService(Context.CLIPBOARD_SERVICE);
        if (clip != null) clip.setPrimaryClip(ClipData.newPlainText("mm0 proof", proofId + "\n" + link));
        Intent send = new Intent(Intent.ACTION_SEND);
        send.setType("text/plain");
        send.putExtra(Intent.EXTRA_TEXT, proofId + "\n" + link);
        try {
            startActivity(Intent.createChooser(send, "Share proof"));
        } catch (android.content.ActivityNotFoundException e) {
            message = "copied to the clipboard";
            render();
        }
    }

    private static void trace(String what, Object... kv) {
        StringBuilder b = new StringBuilder(what);
        for (Object o : kv) b.append(' ').append(o);
        Log.d(TAG, b.toString());
    }
}
