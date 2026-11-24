package com.emthin.pad;

import android.app.Activity;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.hardware.Camera;
import android.os.Bundle;
import android.util.Log;
import android.view.Gravity;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.View;
import android.widget.Button;
import android.widget.FrameLayout;
import android.widget.TextView;

/**
 * The camera half of pairing: decode one QR code and hand its text back.
 *
 * <p>It returns text, and nothing else — the caller parses it with the same
 * {@link PairingLink} and drives the same {@link PairingProtocol} a pasted link
 * does, so scanning can never grant what typing the same string would not.
 *
 * <p>The camera is {@code android.hardware.Camera} rather than Camera2 because
 * the pad is gradle-free and this is the API that needs no extra surface
 * plumbing for a preview + NV21 callback. A device without a camera (the
 * Waydroid lane) and a denied permission both come back as a reason string so
 * the pairing screen can say why and stay usable.
 */
public final class QrScanActivity extends Activity implements SurfaceHolder.Callback,
        Camera.PreviewCallback {
    /** The decoded text, when {@link #RESULT_OK}. */
    static final String EXTRA_TEXT = "qrText";
    /** Why no text came back, when the result is not OK. */
    static final String EXTRA_REASON = "qrReason";
    static final String REASON_NO_CAMERA = "no camera";
    static final String REASON_DENIED = "camera permission denied";
    static final String REASON_FAILED = "camera failed";
    /** What the screen says while the user is away granting the permission. */
    static final String DENIED_STATUS =
            "Camera permission denied \u2014 allow it in Settings, then come back";

    private static final int REQUEST_CAMERA = 41;
    private static final String TAG = "emthin-pad";

    private SurfaceView preview;
    private TextView status;
    private Button settings;
    private Camera camera;
    private boolean decoding;
    private boolean finished;
    /** Kept so a grant that arrives later still has something to preview on. */
    private SurfaceHolder pendingHolder;
    /** The prompt was answered with a denial and the screen is waiting on it. */
    private boolean denied;

    @Override protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        FrameLayout root = new FrameLayout(this);
        preview = new SurfaceView(this);
        preview.getHolder().addCallback(this);
        root.addView(preview, new FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT));
        status = new TextView(this);
        status.setText("Point the camera at the pairing QR code");
        status.setTextColor(0xFFFFFFFF);
        status.setGravity(Gravity.CENTER);
        status.setPadding(24, 24, 24, 24);
        FrameLayout.LayoutParams statusParams = new FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT,
                FrameLayout.LayoutParams.WRAP_CONTENT, Gravity.BOTTOM);
        root.addView(status, statusParams);
        // A denial does not end the screen: the permission can still be granted
        // in Settings and this activity is the state that comes back to.
        settings = new Button(this);
        settings.setText("Open settings");
        settings.setVisibility(View.GONE);
        settings.setOnClickListener(new View.OnClickListener() {
            @Override public void onClick(View v) {
                startActivity(new Intent(android.provider.Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                        android.net.Uri.parse("package:" + getPackageName())));
            }
        });
        root.addView(settings, new FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.WRAP_CONTENT, FrameLayout.LayoutParams.WRAP_CONTENT,
                Gravity.BOTTOM | Gravity.END));
        Button cancel = new Button(this);
        cancel.setText("Cancel");
        cancel.setOnClickListener(new View.OnClickListener() {
            @Override public void onClick(View v) {
                finishWith(null, denied ? REASON_DENIED : null);
            }
        });
        root.addView(cancel, new FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.WRAP_CONTENT, FrameLayout.LayoutParams.WRAP_CONTENT,
                Gravity.TOP | Gravity.END));
        setContentView(root);
        if (checkSelfPermission(android.Manifest.permission.CAMERA)
                != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(new String[] {android.Manifest.permission.CAMERA}, REQUEST_CAMERA);
        }
    }

    @Override public void onRequestPermissionsResult(int request, String[] permissions, int[] results) {
        super.onRequestPermissionsResult(request, permissions, results);
        if (request != REQUEST_CAMERA) return;
        boolean granted = results.length > 0 && results[0] == PackageManager.PERMISSION_GRANTED;
        if (!granted) {
            showDenied();
            return;
        }
        tryOpen();
    }

    /**
     * The permission may have been granted while this screen was in the
     * background (Settings, then back), which delivers no prompt result, so
     * every resume re-asks the same question the callbacks do.
     */
    @Override protected void onResume() {
        super.onResume();
        if (denied && granted()) {
            // Coming back with the grant: say so, and let tryOpen start the
            // preview instead of leaving the denied message up.
            Log.i(TAG, "qr permission granted on return");
            denied = false;
            settings.setVisibility(View.GONE);
            status.setText("Point the camera at the pairing QR code");
        }
        tryOpen();
    }

    // The surface is created as soon as the view is laid out, which is before
    // the permission dialog is answered. Opening here regardless would report
    // a denial as "no camera" (open() throws) and dismiss the screen while the
    // dialog is still up, so the surface is only remembered until the answer.
    @Override public void surfaceCreated(SurfaceHolder holder) {
        pendingHolder = holder;
        tryOpen();
    }

    @Override public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) {
        pendingHolder = holder;
        tryOpen();
    }

    private boolean granted() {
        return checkSelfPermission(android.Manifest.permission.CAMERA)
                == PackageManager.PERMISSION_GRANTED;
    }

    private void tryOpen() {
        if (!QrScan.wantsCamera(finished, granted(), pendingHolder != null, camera != null)) return;
        openCamera(pendingHolder);
    }

    private void showDenied() {
        denied = true;
        status.setText(DENIED_STATUS);
        settings.setVisibility(View.VISIBLE);
        Log.i(TAG, "qr permission denied");
    }

    @Override public void surfaceDestroyed(SurfaceHolder holder) {
        releaseCamera();
    }

    private void openCamera(SurfaceHolder holder) {
        if (camera != null || finished) return;
        Camera opened;
        try {
            opened = Camera.open();
        } catch (RuntimeException e) {
            // No camera backend at all, or one another app holds.
            Log.i(TAG, "qr camera unavailable: " + e.getMessage());
            finishWith(null, REASON_NO_CAMERA);
            return;
        }
        if (opened == null) {
            // A container with no camera HAL answers open() with null rather
            // than throwing; that is the same "no camera" outcome.
            Log.i(TAG, "qr camera unavailable: open() returned null");
            finishWith(null, REASON_NO_CAMERA);
            return;
        }
        try {
            opened.setPreviewDisplay(holder);
            Camera.Parameters parameters = opened.getParameters();
            parameters.setPreviewFormat(android.graphics.ImageFormat.NV21);
            opened.setParameters(parameters);
            opened.setPreviewCallback(this);
            opened.startPreview();
            camera = opened;
        } catch (Exception e) {
            Log.i(TAG, "qr camera failed: " + e.getMessage());
            try {
                opened.release();
            } catch (RuntimeException ignored) {
                // Nothing to release.
            }
            finishWith(null, REASON_FAILED);
        }
    }

    private void releaseCamera() {
        Camera current = camera;
        camera = null;
        if (current == null) return;
        try {
            current.setPreviewCallback(null);
            current.stopPreview();
            current.release();
        } catch (RuntimeException ignored) {
            // The camera is going away either way.
        }
    }

    @Override public void onPreviewFrame(final byte[] frame, Camera source) {
        if (decoding || finished) return;
        decoding = true;
        Camera.Size size = source.getParameters().getPreviewSize();
        final int width = size.width;
        final int height = size.height;
        // The preview buffer is reused by the camera as soon as this callback
        // returns, so the luminance plane is copied before the frame is handed
        // to a worker; decoding a full frame would otherwise race the camera.
        final byte[] luminance = new byte[width * height];
        if (frame.length < luminance.length) {
            decoding = false;
            return;
        }
        System.arraycopy(frame, 0, luminance, 0, luminance.length);
        new Thread(new Runnable() {
            @Override public void run() {
                final String text = QrScan.decodeLuminance(luminance, width, height);
                runOnUiThread(new Runnable() {
                    @Override public void run() {
                        decoding = false;
                        if (text != null) {
                            finishWith(text, null);
                        } else {
                            status.setText("No code yet \u2014 point the camera at the pairing QR");
                        }
                    }
                });
            }
        }, "emthin-qr").start();
    }

    private void finishWith(String text, String reason) {
        if (finished) return;
        finished = true;
        releaseCamera();
        Intent result = new Intent();
        if (text != null) {
            result.putExtra(EXTRA_TEXT, text);
            setResult(RESULT_OK, result);
        } else {
            result.putExtra(EXTRA_REASON, reason == null ? "cancelled" : reason);
            setResult(RESULT_CANCELED, result);
        }
        finish();
    }
}
