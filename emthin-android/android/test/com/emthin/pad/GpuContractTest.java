package com.emthin.pad;

import java.util.Arrays;

/** Host JVM checks for {@link GpuContract}. */
public final class GpuContractTest {
    public static void main(String[] args) {
        String capJson = "{"
                + "\"eglVendor\":\"Mesa Project\","
                + "\"eglRenderer\":\"Adreno X1-85\","
                + "\"eglVersion\":\"1.4\","
                + "\"glesVersion\":\"OpenGL ES 3.2\","
                + "\"hasGles3\":true,"
                + "\"hasEglImageBase\":true,"
                + "\"hasEglAndroidImageNativeBuffer\":true,"
                + "\"hasEglDmabufImport\":false,"
                + "\"hasVulkan\":true,"
                + "\"vulkanDeviceName\":\"Turnip Adreno X1-85\","
                + "\"vulkanDeviceType\":\"integrated\","
                + "\"vulkanApiVersion\":\"1.1.0\","
                + "\"vulkanMaxComputeInvocations\":1024,"
                + "\"hasVkAhb\":true,"
                + "\"hasVkDmabuf\":true"
                + "}";
        GpuContract.Capabilities caps = GpuContract.Capabilities.fromJson(capJson);
        assert caps.hasGles3 : "hasGles3";
        assert caps.hasVulkan : "hasVulkan";
        assert caps.supportsGpuCompute() : "supportsGpuCompute";
        assert caps.summary().contains("Adreno X1-85") : caps.summary();
        assert caps.summary().contains("Vulkan 1.1.0") : caps.summary();

        String compJson = "{"
                + "\"ok\":true,"
                + "\"usedGpu\":true,"
                + "\"backend\":\"vulkan-compute\","
                + "\"device\":\"Turnip Adreno X1-85\","
                + "\"elapsedUs\":420,"
                + "\"outputLen\":12,"
                + "\"output\":[0.0,1.5,4.0]"
                + "}";
        GpuContract.ComputeResult res = GpuContract.ComputeResult.fromJson(compJson);
        assert res.ok && res.usedGpu : "ok+usedGpu";
        assert "vulkan-compute".equals(res.backend) : res.backend;
        assert res.floatValues.length == 3 : Arrays.toString(res.floatValues);
        assert Math.abs(res.floatValues[1] - 1.5f) < 1e-5f;

        float[] rawIn = new float[] {1.5f, -2.0f};
        byte[] rawBytes = GpuContract.floatsToLeBytes(rawIn);
        String spirvJson = "{"
                + "\"ok\":true,"
                + "\"usedGpu\":true,"
                + "\"backend\":\"vulkan-compute\","
                + "\"device\":\"Turnip\","
                + "\"elapsedUs\":300,"
                + "\"outputLen\":" + rawBytes.length + ","
                + "\"outputHex\":\"" + GpuContract.hexEncode(rawBytes) + "\""
                + "}";
        GpuContract.ComputeResult binRes = GpuContract.ComputeResult.fromJson(spirvJson);
        assert Arrays.equals(binRes.bytes, rawBytes) : "hex roundtrip";
        float[] decoded = binRes.bytesAsFloatsLe();
        assert decoded.length == 2 && Math.abs(decoded[0] - 1.5f) < 1e-5f && Math.abs(decoded[1] + 2.0f) < 1e-5f;

        GpuContract.RenderResult render = GpuContract.RenderResult.fromJson(
                "{\"rendered\":true,\"mode\":\"shm\",\"backend\":\"egl-gles3\",\"width\":320,\"height\":200,\"glRenderer\":\"Adreno X1-85\"}");
        assert render.rendered && render.usedGpu() && "shm".equals(render.mode)
                && render.width == 320 && render.height == 200;

        GpuContract.RenderResult cpuRender = GpuContract.RenderResult.fromJson(
                "{\"rendered\":true,\"mode\":\"cpu-shm\",\"backend\":\"cpu-software\",\"width\":320,\"height\":200,\"glRenderer\":\"cpu-software\",\"fallbackReason\":\"forced CPU rendering mode\"}");
        assert cpuRender.rendered && !cpuRender.usedGpu()
                && "cpu-software".equals(cpuRender.backend)
                && "forced CPU rendering mode".equals(cpuRender.fallbackReason);

        GpuContract.Capabilities cpuCaps = GpuContract.Capabilities.fromJson(
                "{\"hasGles3\":false,\"hasVulkan\":false,\"forceCpu\":true,\"cpuFallbackActive\":true,\"cpuFallbackReason\":\"incompatible GPU\"}");
        assert cpuCaps.forceCpu && cpuCaps.cpuFallbackActive && !cpuCaps.supportsGpuCompute();
        assert cpuCaps.summary().contains("CPU fallback") && cpuCaps.summary().contains("incompatible GPU")
                : cpuCaps.summary();

        assert "app:panel".equals(GpuContract.surfaceKey("app", "panel"));
        assert "wayland:f0".equals(GpuContract.surfaceKey("wayland", "f0"));

        byte[] spirv = GpuContract.buildDoubleSpirvBytes();
        assert spirv.length > 40 && (spirv.length % 4) == 0 : "spirv length";
        assert (spirv[0] & 0xff) == 0x03 && (spirv[1] & 0xff) == 0x02
                && (spirv[2] & 0xff) == 0x23 && (spirv[3] & 0xff) == 0x07 : "spirv magic LE";

        System.out.println("GpuContractTest: ok");
    }
}
