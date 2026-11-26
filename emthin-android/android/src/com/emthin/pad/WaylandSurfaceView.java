package com.emthin.pad;

import android.content.Context;
import android.view.Surface;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.View;

/**
 * SurfaceView backing one non-terminal {@code \app} figure in the embedded
 * Wayland compositor. Renders committed client frames ({@code zwp_linux_dmabuf_v1}
 * or {@code wl_shm}) onto its {@link Surface} via the shared app-wide
 * {@link GpuRuntime}.
 */
public final class WaylandSurfaceView extends SurfaceView implements SurfaceHolder.Callback {
    private final long compositorHandle;
    private final String figureKey;
    private volatile Surface currentSurface;
    private volatile boolean destroyed;
    private volatile float u0, v0, u1 = 1f, v1 = 1f;
    private volatile GpuContract.RenderResult lastResult;
    private volatile boolean renderQueued;

    public WaylandSurfaceView(Context context, long compositorHandle, String figureKey) {
        super(context);
        this.compositorHandle = compositorHandle;
        this.figureKey = figureKey;
        setZOrderOnTop(true);
        setClickable(false);
        setFocusable(false);
        setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO);
        getHolder().addCallback(this);
    }

    public String getFigureKey() {
        return figureKey;
    }

    public Surface getCurrentSurface() {
        return currentSurface;
    }

    public boolean hasValidSurface() {
        Surface s = currentSurface;
        return !destroyed && s != null && s.isValid();
    }

    public GpuContract.RenderResult getLastResult() {
        return lastResult;
    }

    public void setCrop(float left, float top, float right, float bottom) {
        float nl = Math.max(0f, Math.min(1f, left));
        float nt = Math.max(0f, Math.min(1f, top));
        float nr = Math.max(nl, Math.min(1f, right));
        float nb = Math.max(nt, Math.min(1f, bottom));
        boolean changed = Math.abs(nl - u0) > 1e-4f || Math.abs(nt - v0) > 1e-4f
                || Math.abs(nr - u1) > 1e-4f || Math.abs(nb - v1) > 1e-4f;
        u0 = nl;
        v0 = nt;
        u1 = nr;
        v1 = nb;
        if (changed) requestFrame();
    }

    public void requestFrame() {
        if (destroyed || compositorHandle == 0 || renderQueued) return;
        final Surface target = currentSurface;
        if (target == null || !target.isValid()) return;
        renderQueued = true;
        GpuRuntime.get().execute(new Runnable() {
            @Override
            public void run() {
                renderQueued = false;
                if (destroyed || target != currentSurface || !target.isValid()) return;
                lastResult = GpuRuntime.get().renderWaylandFigure(
                        compositorHandle, figureKey, target, u0, v0, u1, v1);
            }
        });
    }

    public void release() {
        destroyed = true;
        currentSurface = null;
        GpuRuntime.get().execute(new Runnable() {
            @Override
            public void run() {
                GpuRuntime.get().releaseSurface("wayland", figureKey);
            }
        });
    }

    @Override
    public void surfaceCreated(SurfaceHolder holder) {
        currentSurface = holder.getSurface();
        requestFrame();
    }

    @Override
    public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) {
        currentSurface = holder.getSurface();
        requestFrame();
    }

    @Override
    public void surfaceDestroyed(SurfaceHolder holder) {
        currentSurface = null;
        GpuRuntime.get().execute(new Runnable() {
            @Override
            public void run() {
                GpuRuntime.get().releaseSurface("wayland", figureKey);
            }
        });
    }

    @Override
    protected void onDetachedFromWindow() {
        currentSurface = null;
        super.onDetachedFromWindow();
    }
}
