package com.emthin.pad;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.List;
import java.util.Map;

/**
 * Pure-Java contract types, JSON codecs, and SPIR-V compute helpers for the
 * app-wide GPU runtime ({@link GpuRuntime}). Kept free of {@code android.*}
 * imports so host JVM tests verify the exact payload formats used on Android.
 */
public final class GpuContract {
    private static final int MAX_JSON_CHARS = 512 * 1024;
    private static final char[] HEX = "0123456789abcdef".toCharArray();

    private GpuContract() {}

    public static final class Capabilities {
        public final String eglVendor;
        public final String eglRenderer;
        public final String eglVersion;
        public final String glesVersion;
        public final boolean hasGles3;
        public final boolean hasEglImageBase;
        public final boolean hasEglAndroidImageNativeBuffer;
        public final boolean hasEglDmabufImport;
        public final boolean hasVulkan;
        public final String vulkanDeviceName;
        public final String vulkanDeviceType;
        public final String vulkanApiVersion;
        public final int vulkanMaxComputeInvocations;
        public final boolean hasVkAhb;
        public final boolean hasVkDmabuf;
        public final boolean forceCpu;
        public final boolean cpuFallbackActive;
        public final String cpuFallbackReason;
        public final String probeError;

        private Capabilities(
                String eglVendor,
                String eglRenderer,
                String eglVersion,
                String glesVersion,
                boolean hasGles3,
                boolean hasEglImageBase,
                boolean hasEglAndroidImageNativeBuffer,
                boolean hasEglDmabufImport,
                boolean hasVulkan,
                String vulkanDeviceName,
                String vulkanDeviceType,
                String vulkanApiVersion,
                int vulkanMaxComputeInvocations,
                boolean hasVkAhb,
                boolean hasVkDmabuf,
                boolean forceCpu,
                boolean cpuFallbackActive,
                String cpuFallbackReason,
                String probeError) {
            this.eglVendor = eglVendor;
            this.eglRenderer = eglRenderer;
            this.eglVersion = eglVersion;
            this.glesVersion = glesVersion;
            this.hasGles3 = hasGles3;
            this.hasEglImageBase = hasEglImageBase;
            this.hasEglAndroidImageNativeBuffer = hasEglAndroidImageNativeBuffer;
            this.hasEglDmabufImport = hasEglDmabufImport;
            this.hasVulkan = hasVulkan;
            this.vulkanDeviceName = vulkanDeviceName;
            this.vulkanDeviceType = vulkanDeviceType;
            this.vulkanApiVersion = vulkanApiVersion;
            this.vulkanMaxComputeInvocations = vulkanMaxComputeInvocations;
            this.hasVkAhb = hasVkAhb;
            this.hasVkDmabuf = hasVkDmabuf;
            this.forceCpu = forceCpu;
            this.cpuFallbackActive = cpuFallbackActive;
            this.cpuFallbackReason = cpuFallbackReason;
            this.probeError = probeError;
        }

        public static Capabilities fromJson(String json) {
            if (json == null || json.trim().isEmpty()) {
                return new Capabilities("", "", "", "", false, false, false, false,
                        false, "", "", "", 0, false, false, false, true,
                        "empty capabilities JSON", "empty capabilities JSON");
            }
            Map<String, Object> m = GatewayJson.parseObject(json, MAX_JSON_CHARS);
            return new Capabilities(
                    str(m, "eglVendor"),
                    str(m, "eglRenderer"),
                    str(m, "eglVersion"),
                    str(m, "glesVersion"),
                    bool(m, "hasGles3"),
                    bool(m, "hasEglImageBase"),
                    bool(m, "hasEglAndroidImageNativeBuffer"),
                    bool(m, "hasEglDmabufImport"),
                    bool(m, "hasVulkan"),
                    str(m, "vulkanDeviceName"),
                    str(m, "vulkanDeviceType"),
                    str(m, "vulkanApiVersion"),
                    numInt(m, "vulkanMaxComputeInvocations"),
                    bool(m, "hasVkAhb"),
                    bool(m, "hasVkDmabuf"),
                    bool(m, "forceCpu"),
                    bool(m, "cpuFallbackActive"),
                    str(m, "cpuFallbackReason"),
                    str(m, "probeError"));
        }

        public boolean supportsGpuCompute() {
            return !forceCpu && hasVulkan && vulkanMaxComputeInvocations > 0;
        }

        public String summary() {
            if (cpuFallbackActive) {
                return "CPU fallback"
                        + (cpuFallbackReason.isEmpty() ? "" : " (" + cpuFallbackReason + ")");
            }
            String renderer = !eglRenderer.isEmpty() ? eglRenderer : "no-egl";
            String vk = hasVulkan
                    ? ("Vulkan " + vulkanApiVersion + " (" + vulkanDeviceName + ")")
                    : "no-vulkan";
            return renderer + " | " + (hasGles3 ? "GLES3" : "GLES2") + " | " + vk;
        }
    }

    public static final class ComputeResult {
        public final boolean ok;
        public final boolean usedGpu;
        public final String backend;
        public final String device;
        public final long elapsedUs;
        public final int outputLen;
        public final float[] floatValues;
        public final byte[] bytes;
        public final String error;

        private ComputeResult(
                boolean ok,
                boolean usedGpu,
                String backend,
                String device,
                long elapsedUs,
                int outputLen,
                float[] floatValues,
                byte[] bytes,
                String error) {
            this.ok = ok;
            this.usedGpu = usedGpu;
            this.backend = backend;
            this.device = device;
            this.elapsedUs = elapsedUs;
            this.outputLen = outputLen;
            this.floatValues = floatValues;
            this.bytes = bytes;
            this.error = error;
        }

        public static ComputeResult fromJson(String json) {
            if (json == null || json.trim().isEmpty()) {
                return new ComputeResult(false, false, "error", "", 0L, 0,
                        new float[0], new byte[0], "empty compute result");
            }
            Map<String, Object> m = GatewayJson.parseObject(json, MAX_JSON_CHARS);
            float[] floats = new float[0];
            Object outObj = m.get("output");
            if (outObj instanceof List<?>) {
                List<?> list = (List<?>) outObj;
                floats = new float[list.size()];
                for (int i = 0; i < list.size(); i++) {
                    Object item = list.get(i);
                    floats[i] = item instanceof Number ? ((Number) item).floatValue() : 0f;
                }
            }
            String hex = str(m, "outputHex");
            byte[] decoded = hex.isEmpty() ? new byte[0] : hexDecode(hex);
            return new ComputeResult(
                    bool(m, "ok"),
                    bool(m, "usedGpu"),
                    str(m, "backend"),
                    str(m, "device"),
                    numLong(m, "elapsedUs"),
                    numInt(m, "outputLen"),
                    floats,
                    decoded,
                    str(m, "error"));
        }

        public float[] bytesAsFloatsLe() {
            int count = bytes.length / 4;
            float[] out = new float[count];
            ByteBuffer bb = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN);
            for (int i = 0; i < count; i++) {
                out[i] = bb.getFloat(i * 4);
            }
            return out;
        }
    }

    public static final class RenderResult {
        public final boolean rendered;
        public final String mode;
        public final String backend;
        public final int width;
        public final int height;
        public final String glRenderer;
        public final String fallbackReason;
        public final String error;

        private RenderResult(
                boolean rendered,
                String mode,
                String backend,
                int width,
                int height,
                String glRenderer,
                String fallbackReason,
                String error) {
            this.rendered = rendered;
            this.mode = mode;
            this.backend = backend;
            this.width = width;
            this.height = height;
            this.glRenderer = glRenderer;
            this.fallbackReason = fallbackReason;
            this.error = error;
        }

        public boolean usedGpu() {
            return rendered && !"cpu-software".equals(backend);
        }

        public static RenderResult fromJson(String json) {
            if (json == null || json.trim().isEmpty()) {
                return new RenderResult(false, "error", "", 0, 0, "", "", "empty render result");
            }
            Map<String, Object> m = GatewayJson.parseObject(json, MAX_JSON_CHARS);
            return new RenderResult(
                    bool(m, "rendered"),
                    str(m, "mode"),
                    str(m, "backend"),
                    numInt(m, "width"),
                    numInt(m, "height"),
                    str(m, "glRenderer"),
                    str(m, "fallbackReason"),
                    str(m, "error"));
        }
    }

    public static String surfaceKey(String scope, String id) {
        String cleanScope = (scope == null || scope.isEmpty()) ? "app" : scope;
        String cleanId = id == null ? "" : id;
        return cleanScope + ":" + cleanId;
    }

    public static String hexEncode(byte[] bytes) {
        if (bytes == null || bytes.length == 0) return "";
        char[] chars = new char[bytes.length * 2];
        for (int i = 0; i < bytes.length; i++) {
            int b = bytes[i] & 0xff;
            chars[i * 2] = HEX[b >>> 4];
            chars[i * 2 + 1] = HEX[b & 0x0f];
        }
        return new String(chars);
    }

    public static byte[] hexDecode(String hex) {
        if (hex == null || (hex.length() & 1) != 0) {
            throw new IllegalArgumentException("invalid hex string");
        }
        int len = hex.length() / 2;
        byte[] out = new byte[len];
        for (int i = 0; i < len; i++) {
            int hi = Character.digit(hex.charAt(i * 2), 16);
            int lo = Character.digit(hex.charAt(i * 2 + 1), 16);
            if (hi < 0 || lo < 0) throw new IllegalArgumentException("bad hex digit");
            out[i] = (byte) ((hi << 4) | lo);
        }
        return out;
    }

    public static byte[] floatsToLeBytes(float[] values) {
        ByteBuffer bb = ByteBuffer.allocate(values.length * 4).order(ByteOrder.LITTLE_ENDIAN);
        for (float v : values) bb.putFloat(v);
        return bb.array();
    }

    /**
     * Build a valid SPIR-V 1.0 core compute shader implementing {@code out[i] = in[i] + in[i]}
     * (local workgroup size 1x1x1, descriptor set 0 bindings 0 and 1).
     * Proves any subsystem in Java can supply custom SPIR-V binaries to Vulkan.
     */
    public static byte[] buildDoubleSpirvBytes() {
        final int spirvMagic = 0x07230203;
        final int spirvV10 = 0x00010000;

        final int idMain = 1;
        final int idGid = 2;
        final int idRtF32 = 3;
        final int idBufStruct = 4;
        final int idVarIn = 5;
        final int idVarOut = 6;
        final int idVoid = 7;
        final int idFnVoid = 8;
        final int idU32 = 9;
        final int idI32 = 10;
        final int idF32 = 11;
        final int idV3U32 = 12;
        final int idPtrInV3U32 = 13;
        final int idPtrInU32 = 14;
        final int idPtrUniStruct = 15;
        final int idPtrUniF32 = 16;
        final int idCU32Zero = 17;
        final int idCI32Zero = 18;
        final int idEntry = 19;
        final int idGidXPtr = 20;
        final int idIdx = 21;
        final int idInPtr = 22;
        final int idVal = 23;
        final int idRes = 24;
        final int idOutPtr = 25;
        final int bound = 26;

        int[] w = new int[] {
            spirvMagic, spirvV10, 0, bound, 0,
            // OpCapability Shader
            (2 << 16) | 17, 1,
            // OpMemoryModel Logical GLSL450
            (3 << 16) | 14, 0, 1,
            // OpEntryPoint GLCompute %main "main" %gid
            (6 << 16) | 15, 5, idMain, 0x6e69616d, 0, idGid,
            // OpExecutionMode %main LocalSize 1 1 1
            (6 << 16) | 16, idMain, 17, 1, 1, 1,
            // OpDecorate %rt_f32 ArrayStride 4
            (4 << 16) | 71, idRtF32, 6, 4,
            // OpMemberDecorate %buf 0 Offset 0
            (5 << 16) | 72, idBufStruct, 0, 35, 0,
            // OpDecorate %buf BufferBlock
            (3 << 16) | 71, idBufStruct, 3,
            // Descriptor set / binding / builtin decorations
            (4 << 16) | 71, idVarIn, 34, 0,
            (4 << 16) | 71, idVarIn, 33, 0,
            (4 << 16) | 71, idVarOut, 34, 0,
            (4 << 16) | 71, idVarOut, 33, 1,
            (4 << 16) | 71, idGid, 11, 28,
            // Types & variables
            (2 << 16) | 19, idVoid,
            (3 << 16) | 33, idFnVoid, idVoid,
            (4 << 16) | 21, idU32, 32, 0,
            (4 << 16) | 21, idI32, 32, 1,
            (3 << 16) | 22, idF32, 32,
            (4 << 16) | 23, idV3U32, idU32, 3,
            (4 << 16) | 32, idPtrInV3U32, 1, idV3U32,
            (4 << 16) | 32, idPtrInU32, 1, idU32,
            (3 << 16) | 29, idRtF32, idF32,
            (3 << 16) | 30, idBufStruct, idRtF32,
            (4 << 16) | 32, idPtrUniStruct, 2, idBufStruct,
            (4 << 16) | 32, idPtrUniF32, 2, idF32,
            (4 << 16) | 43, idU32, idCU32Zero, 0,
            (4 << 16) | 43, idI32, idCI32Zero, 0,
            (4 << 16) | 59, idPtrInV3U32, idGid, 1,
            (4 << 16) | 59, idPtrUniStruct, idVarIn, 2,
            (4 << 16) | 59, idPtrUniStruct, idVarOut, 2,
            // Function body: out[idx] = in[idx] + in[idx]
            (5 << 16) | 54, idVoid, idMain, 0, idFnVoid,
            (2 << 16) | 248, idEntry,
            (5 << 16) | 65, idPtrInU32, idGidXPtr, idGid, idCU32Zero,
            (4 << 16) | 61, idU32, idIdx, idGidXPtr,
            (6 << 16) | 65, idPtrUniF32, idInPtr, idVarIn, idCI32Zero, idIdx,
            (4 << 16) | 61, idF32, idVal, idInPtr,
            (5 << 16) | 129, idF32, idRes, idVal, idVal,
            (6 << 16) | 65, idPtrUniF32, idOutPtr, idVarOut, idCI32Zero, idIdx,
            (3 << 16) | 62, idOutPtr, idRes,
            (1 << 16) | 253,
            (1 << 16) | 56
        };

        ByteBuffer bb = ByteBuffer.allocate(w.length * 4).order(ByteOrder.LITTLE_ENDIAN);
        for (int word : w) bb.putInt(word);
        return bb.array();
    }

    private static String str(Map<String, Object> m, String key) {
        Object v = m.get(key);
        return v instanceof String ? (String) v : "";
    }

    private static boolean bool(Map<String, Object> m, String key) {
        return Boolean.TRUE.equals(m.get(key));
    }

    private static int numInt(Map<String, Object> m, String key) {
        Object v = m.get(key);
        return v instanceof Number ? ((Number) v).intValue() : 0;
    }

    private static long numLong(Map<String, Object> m, String key) {
        Object v = m.get(key);
        return v instanceof Number ? ((Number) v).longValue() : 0L;
    }
}
