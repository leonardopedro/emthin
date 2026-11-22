package com.emthin.pad;

import android.app.Activity;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.net.Uri;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.view.View;
import android.widget.Button;
import android.widget.TextView;
import android.widget.Toast;

import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.net.URLEncoder;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * Disposable MM0 proof client for the development testing plan (P0b).
 * It is a separate Activity in the test APK, not part of the shipping pad.
 * Verification and publication stay in the backend; this Activity only picks
 * a .mm0/.mm1 pair, posts the existing routes, and hands the proof ID / link
 * back through the share sheet or clipboard.
 */
public final class ProofClientActivity extends Activity {
    private static final int PICK_MM0 = 1;
    private static final int PICK_MM1 = 2;
    private static final String PREFS = "proof-client";
    private static final String KEY_PROOF_ID = "proof-id";
    private static final String KEY_STATUS = "status";

    private Uri mm0;
    private Uri mm1;
    private String proofId = "";
    private String status = "no proof";
    private final Handler main = new Handler(Looper.getMainLooper());
    private final AtomicInteger generation = new AtomicInteger();
    private TextView stateView;
    private android.widget.LinearLayout layout;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        SharedPreferences saved = getSharedPreferences(PREFS, MODE_PRIVATE);
        proofId = saved.getString(KEY_PROOF_ID, "");
        status = saved.getString(KEY_STATUS, "no proof");

        layout = new android.widget.LinearLayout(this);
        layout.setOrientation(android.widget.LinearLayout.VERTICAL);
        layout.setPadding(24, 24, 24, 24);
        stateView = new TextView(this);
        stateView.setTextSize(15f);
        stateView.setPadding(0, 0, 0, 16);
        layout.addView(stateView);
        setContentView(layout);

        Button pick0 = button("Pick .mm0");
        pick0.setOnClickListener(v -> pick(PICK_MM0, "application/octet-stream"));
        Button pick1 = button("Pick .mm1");
        pick1.setOnClickListener(v -> pick(PICK_MM1, "application/octet-stream"));
        Button verify = button("Verify");
        verify.setOnClickListener(v -> post("/api/v1/verify", false));
        Button publish = button("Submit to test server");
        publish.setOnClickListener(v -> post("/api/v1/publish", true));
        Button share = button("Share ID / link");
        share.setOnClickListener(v -> shareResult());
    }

    private Button button(String label) {
        Button b = new Button(this);
        b.setText(label);
        layout.addView(b);
        return b;
    }

    @Override
    protected void onResume() {
        super.onResume();
        render();
    }

    private void render() {
        stateView.setText("status: " + status
                + "\nproof id: " + (proofId.isEmpty() ? "(none)" : proofId)
                + "\n.mm0: " + (mm0 == null ? "(not picked)" : mm0.getLastPathSegment())
                + "\n.mm1: " + (mm1 == null ? "(not picked)" : mm1.getLastPathSegment()));
    }

    private void pick(int request, String type) {
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
        intent.setType(type);
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        intent.addFlags(Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION
                | Intent.FLAG_GRANT_READ_URI_PERMISSION);
        startActivityForResult(intent, request);
    }

    @Override
    protected void onActivityResult(int request, int code, Intent data) {
        super.onActivityResult(request, code, data);
        if (data == null || data.getData() == null) return;
        Uri uri = data.getData();
        getContentResolver().takePersistableUriPermission(
                uri, Intent.FLAG_GRANT_READ_URI_PERMISSION);
        if (request == PICK_MM0) mm0 = uri;
        if (request == PICK_MM1) mm1 = uri;
        render();
    }

    /** Configured locally; no credentials ever leave the device. */
    private String baseUrl() {
        SharedPreferences p = getSharedPreferences(PREFS, MODE_PRIVATE);
        return p.getString("base-url", "http://127.0.0.1:18919");
    }

    private void post(String route, boolean publish) {
        if (mm0 == null || mm1 == null) {
            Toast.makeText(this, "pick both files first", Toast.LENGTH_SHORT).show();
            return;
        }
        final int op = generation.incrementAndGet();
        final Uri captured0 = mm0;
        final Uri captured1 = mm1;
        new Thread(() -> {
            String body = call(route, captured0, captured1);
            main.post(() -> {
                // A result from an older pair or a recreated Activity cannot
                // overwrite the current view.
                if (op != generation.get()) return;
                if (body == null) {
                    status = "request failed";
                } else {
                    status = publish ? "submitted" : "verified";
                    String id = jsonString(body, "proof");
                    if (!id.isEmpty()) proofId = id;
                    getSharedPreferences(PREFS, MODE_PRIVATE).edit()
                            .putString(KEY_PROOF_ID, proofId)
                            .putString(KEY_STATUS, status).apply();
                }
                render();
            });
        }, "mm0-post").start();
    }

    private String call(String route, Uri spec, Uri proof) {
        HttpURLConnection conn = null;
        try {
            String params = "project=unfer&title=sort%20Prop"
                    + "&mm0=" + URLEncoder.encode(read(spec), "UTF-8")
                    + "&mm1=" + URLEncoder.encode(read(proof), "UTF-8");
            byte[] bytes = params.getBytes(StandardCharsets.UTF_8);
            conn = (HttpURLConnection) new URL(baseUrl() + route).openConnection();
            conn.setRequestMethod("POST");
            conn.setDoOutput(true);
            conn.setRequestProperty("Content-Type", "application/x-www-form-urlencoded");
            conn.setRequestProperty("Accept", "application/json");
            conn.setFixedLengthStreamingMode(bytes.length);
            try (OutputStream out = conn.getOutputStream()) {
                out.write(bytes);
            }
            InputStream in = conn.getResponseCode() < 400
                    ? conn.getInputStream() : conn.getErrorStream();
            return in == null ? null : new String(readAll(in), StandardCharsets.UTF_8);
        } catch (Exception e) {
            return null;
        } finally {
            if (conn != null) conn.disconnect();
        }
    }

    private String read(Uri uri) throws Exception {
        try (InputStream in = getContentResolver().openInputStream(uri)) {
            return new String(readAll(in), StandardCharsets.UTF_8);
        }
    }

    private static byte[] readAll(InputStream in) throws Exception {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        byte[] buf = new byte[8192];
        int n;
        while ((n = in.read(buf)) > 0) out.write(buf, 0, n);
        return out.toByteArray();
    }

    private static String jsonString(String json, String key) {
        String needle = "\"" + key + "\"";
        int at = json.indexOf(needle);
        if (at < 0) return "";
        int colon = json.indexOf(':', at + needle.length());
        if (colon < 0) return "";
        int from = colon + 1;
        while (from < json.length() && Character.isWhitespace(json.charAt(from))) from++;
        if (from >= json.length() || json.charAt(from) != '"') return "";
        int end = json.indexOf('"', from + 1);
        return end < 0 ? "" : json.substring(from + 1, end);
    }

    private void shareResult() {
        if (proofId.isEmpty()) {
            Toast.makeText(this, "no proof id yet", Toast.LENGTH_SHORT).show();
            return;
        }
        String link = baseUrl() + "/proof?project=unfer&id=" + proofId;
        Intent send = new Intent(Intent.ACTION_SEND);
        send.setType("text/plain");
        send.putExtra(Intent.EXTRA_TEXT, proofId + "\n" + link);
        startActivity(Intent.createChooser(send, "Hand proof to emthin"));
        ClipboardManager clip = (ClipboardManager) getSystemService(Context.CLIPBOARD_SERVICE);
        if (clip != null) clip.setPrimaryClip(ClipData.newPlainText("mm0 proof", link));
    }
}
