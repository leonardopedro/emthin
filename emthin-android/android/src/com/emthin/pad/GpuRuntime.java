package com.emthin.pad;

import android.view.Surface;

import java.util.concurrent.Callable;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.concurrent.ThreadFactory;

/**
 * Process-wide GPU runtime exposed to the entire {@code emthin-android} app.
 *
 * <p>Any Activity, View, or background worker can use {@link #get()} to:
 * <ul>
 *   <li>Probe EGL / GLES3 / Vulkan 1.1 / AHardwareBuffer capabilities;</li>
 *   <li>Execute built-in or arbitrary SPIR-V compute kernels on Vulkan 1.1;</li>
 *   <li>Composite RGBA pixel buffers onto any Android {@link Surface} via the
 *       shared EGL/GLES3 context;</li>
 *   <li>Composite embedded Wayland client frames (dma-buf / {@code wl_shm}).</li>
 * </ul>
 */
public final class GpuRuntime {
    private static final GpuRuntime INSTANCE = new GpuRuntime();

    private final ExecutorService worker = Executors.newSingleThreadExecutor(new ThreadFactory() {
        @Override
        public Thread newThread(Runnable r) {
            Thread t = new Thread(r, "emthin-gpu-worker");
            t.setDaemon(true);
            return t;
        }
    });

    private volatile GpuContract.Capabilities cachedCaps;

    private GpuRuntime() {}

    public static GpuRuntime get() {
        return INSTANCE;
    }

    public void execute(Runnable task) {
        worker.execute(task);
    }

    public <T> Future<T> submit(Callable<T> task) {
        return worker.submit(task);
    }

    public String capabilitiesJson() {
        return DocBridge.gpuCapabilities();
    }

    public GpuContract.Capabilities capabilities() {
        GpuContract.Capabilities current = cachedCaps;
        if (current != null) return current;
        return refreshCapabilities();
    }

    public GpuContract.Capabilities refreshCapabilities() {
        GpuContract.Capabilities probed = GpuContract.Capabilities.fromJson(DocBridge.gpuCapabilities());
        cachedCaps = probed;
        return probed;
    }

    public String summary() {
        return capabilities().summary();
    }

    public void setForceCpu(boolean force) {
        DocBridge.gpuSetForceCpu(force);
        cachedCaps = null;
    }

    public boolean isCpuFallback() {
        return DocBridge.gpuIsCpuFallback();
    }

    public String computeSmokeJson() {
        return DocBridge.gpuComputeSmoke();
    }

    public GpuContract.ComputeResult computeF32(String kernel, float[] input) {
        return GpuContract.ComputeResult.fromJson(DocBridge.gpuComputeF32(kernel, input));
    }

    public GpuContract.ComputeResult dispatchSpirv(
            byte[] spirv, byte[] input, int outputBytes, int gx, int gy, int gz) {
        return GpuContract.ComputeResult.fromJson(
                DocBridge.gpuDispatchSpirv(spirv, input, outputBytes, gx, gy, gz));
    }

    public GpuContract.RenderResult renderRgba(
            String scope, String id, Surface surface,
            byte[] rgba, int width, int height, int stride,
            float u0, float v0, float u1, float v1) {
        String key = GpuContract.surfaceKey(scope, id);
        return GpuContract.RenderResult.fromJson(
                DocBridge.gpuRenderRgba(key, surface, rgba, width, height, stride, u0, v0, u1, v1));
    }

    public GpuContract.RenderResult renderPlaceholder(String scope, String id, Surface surface) {
        String key = GpuContract.surfaceKey(scope, id);
        return GpuContract.RenderResult.fromJson(DocBridge.gpuRenderPlaceholder(key, surface));
    }

    public void releaseSurface(String scope, String id) {
        DocBridge.gpuReleaseSurface(GpuContract.surfaceKey(scope, id));
    }

    public GpuContract.RenderResult renderWaylandFigure(
            long compositorHandle, String figureKey, Surface surface,
            float u0, float v0, float u1, float v1) {
        return GpuContract.RenderResult.fromJson(
                DocBridge.waylandRenderFigure(compositorHandle, figureKey, surface, u0, v0, u1, v1));
    }
}
