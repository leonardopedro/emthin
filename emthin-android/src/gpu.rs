//! GPU rendering and compute backend for the Android embedded Wayland compositor.
//!
//! Provides:
//! - EGL 1.4 / OpenGL ES 3.x window compositing onto Android `ANativeWindow`
//!   surfaces (`WaylandSurfaceView`) with UV cropping for page viewport clipping.
//! - Zero-copy Wayland `zwp_linux_dmabuf_v1` buffer import (`EGL_LINUX_DMA_BUF_EXT`
//!   + `GL_TEXTURE_EXTERNAL_OES`) and `AHardwareBuffer` (`EGL_ANDROID_image_native_buffer`)
//!   interop, plus fallback `wl_shm` texture upload.
//! - Runtime-loaded Vulkan 1.1 compute shader dispatch (valid SPIR-V 1.0 binary)
//!   and `VK_ANDROID_external_memory_android_hardware_buffer` capability/property
//!   interoperation.

use serde::Serialize;
use std::os::fd::OwnedFd;

pub const DRM_FORMAT_ARGB8888: u32 = 0x3432_5241;
pub const DRM_FORMAT_XRGB8888: u32 = 0x3432_5258;
pub const DRM_FORMAT_ABGR8888: u32 = 0x3432_4241;
pub const DRM_FORMAT_XBGR8888: u32 = 0x3432_4258;

pub const WL_SHM_FORMAT_ARGB8888: u32 = 0;
pub const WL_SHM_FORMAT_XRGB8888: u32 = 1;
/// Wayland fourcc for RGBA8888 bytes in little-endian memory (`ABGR8888`).
pub const WL_SHM_FORMAT_ABGR8888: u32 = DRM_FORMAT_ABGR8888;

/// One plane of an imported `zwp_linux_dmabuf_v1` buffer.
#[derive(Debug)]
pub struct DmabufPlane {
    pub fd: OwnedFd,
    pub plane_idx: u32,
    pub offset: u32,
    pub stride: u32,
    pub modifier: u64,
}

/// Complete Linux dma-buf descriptor received from a Wayland client.
#[derive(Debug)]
pub struct DmabufDescriptor {
    pub width: u32,
    pub height: u32,
    pub drm_format: u32,
    pub flags: u32,
    pub planes: Vec<DmabufPlane>,
}

/// Borrowed `wl_shm` frame ready for GL texture upload.
#[derive(Debug, Clone, Copy)]
pub struct ShmFrameRef<'a> {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub wl_format: u32,
    pub pixels: &'a [u8],
}

/// Probed EGL / GLES / Vulkan capabilities on the active device.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuCapabilities {
    pub egl_vendor: String,
    pub egl_renderer: String,
    pub egl_version: String,
    pub gles_version: String,
    pub has_gles3: bool,
    pub has_egl_image_base: bool,
    pub has_egl_android_image_native_buffer: bool,
    pub has_egl_dmabuf_import: bool,
    pub has_egl_dmabuf_modifiers: bool,
    pub has_egl_native_fence_sync: bool,
    pub has_gl_oes_egl_image_external: bool,
    pub has_vulkan: bool,
    pub vulkan_device_name: String,
    pub vulkan_device_type: String,
    pub vulkan_api_version: String,
    pub vulkan_compute_queue_family: Option<u32>,
    pub vulkan_max_compute_invocations: u32,
    pub has_vk_ahb: bool,
    pub has_vk_dmabuf: bool,
    pub force_cpu: bool,
    pub cpu_fallback_active: bool,
    pub cpu_fallback_reason: Option<String>,
    pub probe_error: Option<String>,
}

/// Outcome of rendering one figure or app frame to an Android `Surface`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderResult {
    pub rendered: bool,
    pub mode: &'static str,
    pub backend: &'static str,
    pub width: u32,
    pub height: u32,
    pub gl_renderer: String,
    pub fallback_reason: Option<String>,
}

/// Outcome of the Vulkan compute + `AHardwareBuffer` interop smoke check.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputeSmokeResult {
    pub ok: bool,
    pub backend: String,
    pub device: String,
    pub sample_ok: bool,
    pub ahb_egl_ok: bool,
    pub ahb_vulkan_ok: bool,
    pub elapsed_us: u64,
    pub error: Option<String>,
}

/// Outcome of a general-purpose GPU compute kernel or custom SPIR-V dispatch.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputeDispatchResult {
    pub ok: bool,
    pub used_gpu: bool,
    pub backend: String,
    pub device: String,
    pub elapsed_us: u64,
    pub output_len: usize,
    pub error: Option<String>,
}

/// Emit a valid SPIR-V 1.0 compute shader computing `out[i] = in[i] * in[i] + 1.0`.
///
/// Layout:
/// - `DescriptorSet 0, Binding 0`: `readonly/write` storage buffer `{ float[] }`
/// - `DescriptorSet 0, Binding 1`: storage buffer `{ float[] }`
/// - `LocalSize = (64, 1, 1)`
pub fn build_square_plus_one_spirv() -> Vec<u32> {
    const SPIRV_MAGIC: u32 = 0x0723_0203;
    const SPIRV_V1_0: u32 = 0x0001_0000;

    // OpCodes
    const OP_CAPABILITY: u16 = 17;
    const OP_MEMORY_MODEL: u16 = 14;
    const OP_ENTRY_POINT: u16 = 15;
    const OP_EXECUTION_MODE: u16 = 16;
    const OP_DECORATE: u16 = 71;
    const OP_MEMBER_DECORATE: u16 = 72;
    const OP_TYPE_VOID: u16 = 19;
    const OP_TYPE_INT: u16 = 21;
    const OP_TYPE_FLOAT: u16 = 22;
    const OP_TYPE_VECTOR: u16 = 23;
    const OP_TYPE_RUNTIME_ARRAY: u16 = 29;
    const OP_TYPE_STRUCT: u16 = 30;
    const OP_TYPE_POINTER: u16 = 32;
    const OP_TYPE_FUNCTION: u16 = 33;
    const OP_CONSTANT: u16 = 43;
    const OP_VARIABLE: u16 = 59;
    const OP_LOAD: u16 = 61;
    const OP_STORE: u16 = 62;
    const OP_ACCESS_CHAIN: u16 = 65;
    const OP_FUNCTION: u16 = 54;
    const OP_FUNCTION_END: u16 = 56;
    const OP_FADD: u16 = 129;
    const OP_FMUL: u16 = 133;
    const OP_LABEL: u16 = 248;
    const OP_RETURN: u16 = 253;

    // SPIR-V enums
    const CAP_SHADER: u32 = 1;
    const ADDR_LOGICAL: u32 = 0;
    const MEM_GLSL450: u32 = 1;
    const EXEC_GL_COMPUTE: u32 = 5;
    const MODE_LOCAL_SIZE: u32 = 17;
    const DEC_BUFFER_BLOCK: u32 = 3;
    const DEC_ARRAY_STRIDE: u32 = 6;
    const DEC_BUILTIN: u32 = 11;
    const DEC_BINDING: u32 = 33;
    const DEC_DESCRIPTOR_SET: u32 = 34;
    const DEC_OFFSET: u32 = 35;
    const BUILTIN_GLOBAL_INVOCATION_ID: u32 = 28;
    const SC_INPUT: u32 = 1;
    const SC_UNIFORM: u32 = 2;

    // Result IDs
    const ID_MAIN: u32 = 1;
    const ID_GID: u32 = 2;
    const ID_RT_F32: u32 = 3;
    const ID_BUF_STRUCT: u32 = 4;
    const ID_VAR_IN: u32 = 5;
    const ID_VAR_OUT: u32 = 6;
    const ID_VOID: u32 = 7;
    const ID_FN_VOID: u32 = 8;
    const ID_U32: u32 = 9;
    const ID_I32: u32 = 10;
    const ID_F32: u32 = 11;
    const ID_V3U32: u32 = 12;
    const ID_PTR_IN_V3U32: u32 = 13;
    const ID_PTR_IN_U32: u32 = 14;
    const ID_PTR_UNI_STRUCT: u32 = 15;
    const ID_PTR_UNI_F32: u32 = 16;
    const ID_C_U32_0: u32 = 17;
    const ID_C_I32_0: u32 = 18;
    const ID_C_F32_1: u32 = 19;
    const ID_ENTRY: u32 = 20;
    const ID_GID_X_PTR: u32 = 21;
    const ID_IDX: u32 = 22;
    const ID_IN_PTR: u32 = 23;
    const ID_VAL: u32 = 24;
    const ID_SQ: u32 = 25;
    const ID_RES: u32 = 26;
    const ID_OUT_PTR: u32 = 27;
    const BOUND: u32 = 28;

    let mut w = vec![SPIRV_MAGIC, SPIRV_V1_0, 0, BOUND, 0];
    let push = |w: &mut Vec<u32>, opcode: u16, ops: &[u32]| {
        let count = (ops.len() + 1) as u32;
        w.push((count << 16) | opcode as u32);
        w.extend_from_slice(ops);
    };

    // Capabilities & memory model
    push(&mut w, OP_CAPABILITY, &[CAP_SHADER]);
    push(&mut w, OP_MEMORY_MODEL, &[ADDR_LOGICAL, MEM_GLSL450]);

    // Entry point ("main\0" padded to 2 words) & execution mode
    let main_w0 = u32::from_le_bytes(*b"main");
    push(
        &mut w,
        OP_ENTRY_POINT,
        &[EXEC_GL_COMPUTE, ID_MAIN, main_w0, 0, ID_GID],
    );
    push(
        &mut w,
        OP_EXECUTION_MODE,
        &[ID_MAIN, MODE_LOCAL_SIZE, 64, 1, 1],
    );

    // Decorations
    push(&mut w, OP_DECORATE, &[ID_RT_F32, DEC_ARRAY_STRIDE, 4]);
    push(
        &mut w,
        OP_MEMBER_DECORATE,
        &[ID_BUF_STRUCT, 0, DEC_OFFSET, 0],
    );
    push(&mut w, OP_DECORATE, &[ID_BUF_STRUCT, DEC_BUFFER_BLOCK]);
    push(&mut w, OP_DECORATE, &[ID_VAR_IN, DEC_DESCRIPTOR_SET, 0]);
    push(&mut w, OP_DECORATE, &[ID_VAR_IN, DEC_BINDING, 0]);
    push(&mut w, OP_DECORATE, &[ID_VAR_OUT, DEC_DESCRIPTOR_SET, 0]);
    push(&mut w, OP_DECORATE, &[ID_VAR_OUT, DEC_BINDING, 1]);
    push(
        &mut w,
        OP_DECORATE,
        &[ID_GID, DEC_BUILTIN, BUILTIN_GLOBAL_INVOCATION_ID],
    );

    // Types, constants, global variables
    push(&mut w, OP_TYPE_VOID, &[ID_VOID]);
    push(&mut w, OP_TYPE_FUNCTION, &[ID_FN_VOID, ID_VOID]);
    push(&mut w, OP_TYPE_INT, &[ID_U32, 32, 0]);
    push(&mut w, OP_TYPE_INT, &[ID_I32, 32, 1]);
    push(&mut w, OP_TYPE_FLOAT, &[ID_F32, 32]);
    push(&mut w, OP_TYPE_VECTOR, &[ID_V3U32, ID_U32, 3]);
    push(
        &mut w,
        OP_TYPE_POINTER,
        &[ID_PTR_IN_V3U32, SC_INPUT, ID_V3U32],
    );
    push(&mut w, OP_TYPE_POINTER, &[ID_PTR_IN_U32, SC_INPUT, ID_U32]);
    push(&mut w, OP_TYPE_RUNTIME_ARRAY, &[ID_RT_F32, ID_F32]);
    push(&mut w, OP_TYPE_STRUCT, &[ID_BUF_STRUCT, ID_RT_F32]);
    push(
        &mut w,
        OP_TYPE_POINTER,
        &[ID_PTR_UNI_STRUCT, SC_UNIFORM, ID_BUF_STRUCT],
    );
    push(
        &mut w,
        OP_TYPE_POINTER,
        &[ID_PTR_UNI_F32, SC_UNIFORM, ID_F32],
    );
    push(&mut w, OP_CONSTANT, &[ID_U32, ID_C_U32_0, 0]);
    push(&mut w, OP_CONSTANT, &[ID_I32, ID_C_I32_0, 0]);
    push(&mut w, OP_CONSTANT, &[ID_F32, ID_C_F32_1, 1.0f32.to_bits()]);
    push(&mut w, OP_VARIABLE, &[ID_PTR_IN_V3U32, ID_GID, SC_INPUT]);
    push(
        &mut w,
        OP_VARIABLE,
        &[ID_PTR_UNI_STRUCT, ID_VAR_IN, SC_UNIFORM],
    );
    push(
        &mut w,
        OP_VARIABLE,
        &[ID_PTR_UNI_STRUCT, ID_VAR_OUT, SC_UNIFORM],
    );

    // Function body: out[idx] = in[idx] * in[idx] + 1.0
    push(&mut w, OP_FUNCTION, &[ID_VOID, ID_MAIN, 0, ID_FN_VOID]);
    push(&mut w, OP_LABEL, &[ID_ENTRY]);
    push(
        &mut w,
        OP_ACCESS_CHAIN,
        &[ID_PTR_IN_U32, ID_GID_X_PTR, ID_GID, ID_C_U32_0],
    );
    push(&mut w, OP_LOAD, &[ID_U32, ID_IDX, ID_GID_X_PTR]);
    push(
        &mut w,
        OP_ACCESS_CHAIN,
        &[ID_PTR_UNI_F32, ID_IN_PTR, ID_VAR_IN, ID_C_I32_0, ID_IDX],
    );
    push(&mut w, OP_LOAD, &[ID_F32, ID_VAL, ID_IN_PTR]);
    push(&mut w, OP_FMUL, &[ID_F32, ID_SQ, ID_VAL, ID_VAL]);
    push(&mut w, OP_FADD, &[ID_F32, ID_RES, ID_SQ, ID_C_F32_1]);
    push(
        &mut w,
        OP_ACCESS_CHAIN,
        &[ID_PTR_UNI_F32, ID_OUT_PTR, ID_VAR_OUT, ID_C_I32_0, ID_IDX],
    );
    push(&mut w, OP_STORE, &[ID_OUT_PTR, ID_RES]);
    push(&mut w, OP_RETURN, &[]);
    push(&mut w, OP_FUNCTION_END, &[]);

    w
}

/// Basic structural sanity validator for generated SPIR-V streams.
pub fn validate_spirv_structure(words: &[u32]) -> Result<(), String> {
    if words.len() < 5 {
        return Err("SPIR-V stream shorter than 5-word header".into());
    }
    if words[0] != 0x0723_0203 {
        return Err(format!("bad SPIR-V magic: 0x{:08x}", words[0]));
    }
    let bound = words[3];
    if bound < 2 {
        return Err("SPIR-V ID bound must be >= 2".into());
    }
    let mut cursor = 5;
    let mut saw_func = false;
    let mut saw_end = false;
    while cursor < words.len() {
        let word0 = words[cursor];
        let count = (word0 >> 16) as usize;
        let opcode = (word0 & 0xffff) as u16;
        if count == 0 || cursor + count > words.len() {
            return Err(format!(
                "invalid instruction word count {count} at {cursor}"
            ));
        }
        if opcode == 54 {
            saw_func = true;
        } else if opcode == 56 {
            saw_end = true;
        }
        cursor += count;
    }
    if !saw_func || !saw_end {
        return Err("SPIR-V stream missing OpFunction / OpFunctionEnd".into());
    }
    Ok(())
}

/// CPU reference matching `build_square_plus_one_spirv` for verification.
pub fn cpu_square_plus_one(input: &[f32]) -> Vec<f32> {
    input.iter().map(|v| v * v + 1.0).collect()
}

/// Emit a valid SPIR-V 1.0 compute shader computing `out[i] = max(in[i], 0.0)`.
///
/// Uses only SPIR-V 1.0 core instructions (`OpFOrdGreaterThan` + `OpSelect`)
/// with descriptor set 0 bindings 0 (`float[]` input) and 1 (`float[]` output).
pub fn build_relu_spirv() -> Vec<u32> {
    const SPIRV_MAGIC: u32 = 0x0723_0203;
    const SPIRV_V1_0: u32 = 0x0001_0000;

    const OP_CAPABILITY: u16 = 17;
    const OP_MEMORY_MODEL: u16 = 14;
    const OP_ENTRY_POINT: u16 = 15;
    const OP_EXECUTION_MODE: u16 = 16;
    const OP_DECORATE: u16 = 71;
    const OP_MEMBER_DECORATE: u16 = 72;
    const OP_TYPE_VOID: u16 = 19;
    const OP_TYPE_BOOL: u16 = 20;
    const OP_TYPE_INT: u16 = 21;
    const OP_TYPE_FLOAT: u16 = 22;
    const OP_TYPE_VECTOR: u16 = 23;
    const OP_TYPE_RUNTIME_ARRAY: u16 = 29;
    const OP_TYPE_STRUCT: u16 = 30;
    const OP_TYPE_POINTER: u16 = 32;
    const OP_TYPE_FUNCTION: u16 = 33;
    const OP_CONSTANT: u16 = 43;
    const OP_VARIABLE: u16 = 59;
    const OP_LOAD: u16 = 61;
    const OP_STORE: u16 = 62;
    const OP_ACCESS_CHAIN: u16 = 65;
    const OP_FUNCTION: u16 = 54;
    const OP_FUNCTION_END: u16 = 56;
    const OP_SELECT: u16 = 169;
    const OP_FORD_GREATER_THAN: u16 = 186;
    const OP_LABEL: u16 = 248;
    const OP_RETURN: u16 = 253;

    const CAP_SHADER: u32 = 1;
    const ADDR_LOGICAL: u32 = 0;
    const MEM_GLSL450: u32 = 1;
    const EXEC_GL_COMPUTE: u32 = 5;
    const MODE_LOCAL_SIZE: u32 = 17;
    const DEC_BUFFER_BLOCK: u32 = 3;
    const DEC_ARRAY_STRIDE: u32 = 6;
    const DEC_BUILTIN: u32 = 11;
    const DEC_BINDING: u32 = 33;
    const DEC_DESCRIPTOR_SET: u32 = 34;
    const DEC_OFFSET: u32 = 35;
    const BUILTIN_GLOBAL_INVOCATION_ID: u32 = 28;
    const SC_INPUT: u32 = 1;
    const SC_UNIFORM: u32 = 2;

    const ID_MAIN: u32 = 1;
    const ID_GID: u32 = 2;
    const ID_RT_F32: u32 = 3;
    const ID_BUF_STRUCT: u32 = 4;
    const ID_VAR_IN: u32 = 5;
    const ID_VAR_OUT: u32 = 6;
    const ID_VOID: u32 = 7;
    const ID_FN_VOID: u32 = 8;
    const ID_U32: u32 = 9;
    const ID_I32: u32 = 10;
    const ID_F32: u32 = 11;
    const ID_BOOL: u32 = 12;
    const ID_V3U32: u32 = 13;
    const ID_PTR_IN_V3U32: u32 = 14;
    const ID_PTR_IN_U32: u32 = 15;
    const ID_PTR_UNI_STRUCT: u32 = 16;
    const ID_PTR_UNI_F32: u32 = 17;
    const ID_C_U32_0: u32 = 18;
    const ID_C_I32_0: u32 = 19;
    const ID_C_F32_0: u32 = 20;
    const ID_ENTRY: u32 = 21;
    const ID_GID_X_PTR: u32 = 22;
    const ID_IDX: u32 = 23;
    const ID_IN_PTR: u32 = 24;
    const ID_VAL: u32 = 25;
    const ID_COND: u32 = 26;
    const ID_RES: u32 = 27;
    const ID_OUT_PTR: u32 = 28;
    const BOUND: u32 = 29;

    let mut w = vec![SPIRV_MAGIC, SPIRV_V1_0, 0, BOUND, 0];
    let push = |w: &mut Vec<u32>, opcode: u16, ops: &[u32]| {
        let count = (ops.len() + 1) as u32;
        w.push((count << 16) | opcode as u32);
        w.extend_from_slice(ops);
    };

    push(&mut w, OP_CAPABILITY, &[CAP_SHADER]);
    push(&mut w, OP_MEMORY_MODEL, &[ADDR_LOGICAL, MEM_GLSL450]);

    let main_w0 = u32::from_le_bytes(*b"main");
    push(
        &mut w,
        OP_ENTRY_POINT,
        &[EXEC_GL_COMPUTE, ID_MAIN, main_w0, 0, ID_GID],
    );
    push(
        &mut w,
        OP_EXECUTION_MODE,
        &[ID_MAIN, MODE_LOCAL_SIZE, 1, 1, 1],
    );

    push(&mut w, OP_DECORATE, &[ID_RT_F32, DEC_ARRAY_STRIDE, 4]);
    push(
        &mut w,
        OP_MEMBER_DECORATE,
        &[ID_BUF_STRUCT, 0, DEC_OFFSET, 0],
    );
    push(&mut w, OP_DECORATE, &[ID_BUF_STRUCT, DEC_BUFFER_BLOCK]);
    push(&mut w, OP_DECORATE, &[ID_VAR_IN, DEC_DESCRIPTOR_SET, 0]);
    push(&mut w, OP_DECORATE, &[ID_VAR_IN, DEC_BINDING, 0]);
    push(&mut w, OP_DECORATE, &[ID_VAR_OUT, DEC_DESCRIPTOR_SET, 0]);
    push(&mut w, OP_DECORATE, &[ID_VAR_OUT, DEC_BINDING, 1]);
    push(
        &mut w,
        OP_DECORATE,
        &[ID_GID, DEC_BUILTIN, BUILTIN_GLOBAL_INVOCATION_ID],
    );

    push(&mut w, OP_TYPE_VOID, &[ID_VOID]);
    push(&mut w, OP_TYPE_FUNCTION, &[ID_FN_VOID, ID_VOID]);
    push(&mut w, OP_TYPE_BOOL, &[ID_BOOL]);
    push(&mut w, OP_TYPE_INT, &[ID_U32, 32, 0]);
    push(&mut w, OP_TYPE_INT, &[ID_I32, 32, 1]);
    push(&mut w, OP_TYPE_FLOAT, &[ID_F32, 32]);
    push(&mut w, OP_TYPE_VECTOR, &[ID_V3U32, ID_U32, 3]);
    push(
        &mut w,
        OP_TYPE_POINTER,
        &[ID_PTR_IN_V3U32, SC_INPUT, ID_V3U32],
    );
    push(&mut w, OP_TYPE_POINTER, &[ID_PTR_IN_U32, SC_INPUT, ID_U32]);
    push(&mut w, OP_TYPE_RUNTIME_ARRAY, &[ID_RT_F32, ID_F32]);
    push(&mut w, OP_TYPE_STRUCT, &[ID_BUF_STRUCT, ID_RT_F32]);
    push(
        &mut w,
        OP_TYPE_POINTER,
        &[ID_PTR_UNI_STRUCT, SC_UNIFORM, ID_BUF_STRUCT],
    );
    push(
        &mut w,
        OP_TYPE_POINTER,
        &[ID_PTR_UNI_F32, SC_UNIFORM, ID_F32],
    );
    push(&mut w, OP_CONSTANT, &[ID_U32, ID_C_U32_0, 0]);
    push(&mut w, OP_CONSTANT, &[ID_I32, ID_C_I32_0, 0]);
    push(&mut w, OP_CONSTANT, &[ID_F32, ID_C_F32_0, 0.0f32.to_bits()]);
    push(&mut w, OP_VARIABLE, &[ID_PTR_IN_V3U32, ID_GID, SC_INPUT]);
    push(
        &mut w,
        OP_VARIABLE,
        &[ID_PTR_UNI_STRUCT, ID_VAR_IN, SC_UNIFORM],
    );
    push(
        &mut w,
        OP_VARIABLE,
        &[ID_PTR_UNI_STRUCT, ID_VAR_OUT, SC_UNIFORM],
    );

    push(&mut w, OP_FUNCTION, &[ID_VOID, ID_MAIN, 0, ID_FN_VOID]);
    push(&mut w, OP_LABEL, &[ID_ENTRY]);
    push(
        &mut w,
        OP_ACCESS_CHAIN,
        &[ID_PTR_IN_U32, ID_GID_X_PTR, ID_GID, ID_C_U32_0],
    );
    push(&mut w, OP_LOAD, &[ID_U32, ID_IDX, ID_GID_X_PTR]);
    push(
        &mut w,
        OP_ACCESS_CHAIN,
        &[ID_PTR_UNI_F32, ID_IN_PTR, ID_VAR_IN, ID_C_I32_0, ID_IDX],
    );
    push(&mut w, OP_LOAD, &[ID_F32, ID_VAL, ID_IN_PTR]);
    push(
        &mut w,
        OP_FORD_GREATER_THAN,
        &[ID_BOOL, ID_COND, ID_VAL, ID_C_F32_0],
    );
    push(
        &mut w,
        OP_SELECT,
        &[ID_F32, ID_RES, ID_COND, ID_VAL, ID_C_F32_0],
    );
    push(
        &mut w,
        OP_ACCESS_CHAIN,
        &[ID_PTR_UNI_F32, ID_OUT_PTR, ID_VAR_OUT, ID_C_I32_0, ID_IDX],
    );
    push(&mut w, OP_STORE, &[ID_OUT_PTR, ID_RES]);
    push(&mut w, OP_RETURN, &[]);
    push(&mut w, OP_FUNCTION_END, &[]);

    w
}

/// CPU reference matching `build_relu_spirv` for verification and fallback.
pub fn cpu_relu(input: &[f32]) -> Vec<f32> {
    input
        .iter()
        .map(|&v| if v > 0.0 { v } else { 0.0 })
        .collect()
}

/// Software-blit a Wayland SHM or raw RGBA8888 frame into an `ANativeWindow` RGBA8888
/// pixel buffer with UV crop and optional BGRA/XRGB conversion.
pub fn cpu_blit_rgba(
    dst: &mut [u8],
    dst_w: usize,
    dst_h: usize,
    dst_stride_px: usize,
    src: ShmFrameRef<'_>,
    crop: [f32; 4],
) -> Result<(), String> {
    if src.width == 0 || src.height == 0 || src.stride < src.width * 4 {
        return Err("invalid source frame dimensions".into());
    }
    let src_needed = src.stride as usize * src.height as usize;
    if src.pixels.len() < src_needed {
        return Err("truncated source pixel buffer".into());
    }
    if dst_w == 0 || dst_h == 0 || dst_stride_px < dst_w {
        return Err("invalid destination buffer dimensions".into());
    }
    let dst_needed = dst_stride_px * dst_h * 4;
    if dst.len() < dst_needed {
        return Err("truncated destination buffer".into());
    }

    let u0 = crop[0].clamp(0.0, 1.0);
    let v0 = crop[1].clamp(0.0, 1.0);
    let u1 = crop[2].clamp(u0, 1.0);
    let v1 = crop[3].clamp(v0, 1.0);
    let u_span = u1 - u0;
    let v_span = v1 - v0;
    let swap_rb = matches!(
        src.wl_format,
        WL_SHM_FORMAT_ARGB8888 | WL_SHM_FORMAT_XRGB8888
    );
    let opaque = src.wl_format == WL_SHM_FORMAT_XRGB8888;
    let src_w_f = src.width as f32;
    let src_h_f = src.height as f32;
    let max_sx = src.width as usize - 1;
    let max_sy = src.height as usize - 1;

    for dy in 0..dst_h {
        let v = v0 + ((dy as f32 + 0.5) / dst_h as f32) * v_span;
        let sy = ((v * src_h_f) as usize).min(max_sy);
        let src_row = sy * src.stride as usize;
        let dst_row = dy * dst_stride_px * 4;
        for dx in 0..dst_w {
            let u = u0 + ((dx as f32 + 0.5) / dst_w as f32) * u_span;
            let sx = ((u * src_w_f) as usize).min(max_sx);
            let s_off = src_row + sx * 4;
            let d_off = dst_row + dx * 4;
            let b0 = src.pixels[s_off];
            let b1 = src.pixels[s_off + 1];
            let b2 = src.pixels[s_off + 2];
            let a = if opaque { 255 } else { src.pixels[s_off + 3] };
            if swap_rb {
                dst[d_off] = b2;
                dst[d_off + 1] = b1;
                dst[d_off + 2] = b0;
            } else {
                dst[d_off] = b0;
                dst[d_off + 1] = b1;
                dst[d_off + 2] = b2;
            }
            dst[d_off + 3] = a;
        }
    }
    Ok(())
}

/// Fill an `ANativeWindow` RGBA8888 buffer with a dark slate placeholder frame.
pub fn cpu_fill_placeholder(dst: &mut [u8], dst_w: usize, dst_h: usize, dst_stride_px: usize) {
    if dst_w == 0 || dst_h == 0 || dst_stride_px < dst_w {
        return;
    }
    if dst.len() < dst_stride_px * dst_h * 4 {
        return;
    }
    for y in 0..dst_h {
        let border_y = y < 2 || y + 2 >= dst_h;
        let row = y * dst_stride_px * 4;
        for x in 0..dst_w {
            let border = border_y || x < 2 || x + 2 >= dst_w;
            let off = row + x * 4;
            if border {
                dst[off] = 46;
                dst[off + 1] = 64;
                dst[off + 2] = 82;
                dst[off + 3] = 255;
            } else {
                dst[off] = 20;
                dst[off + 1] = 26;
                dst[off + 2] = 31;
                dst[off + 3] = 255;
            }
        }
    }
}

pub fn probe_capabilities() -> GpuCapabilities {
    #[cfg(target_os = "android")]
    {
        android_impl::probe_capabilities()
    }
    #[cfg(not(target_os = "android"))]
    {
        GpuCapabilities {
            probe_error: Some("host build (no Android EGL/Vulkan context)".into()),
            ..Default::default()
        }
    }
}

pub fn compute_smoke() -> ComputeSmokeResult {
    #[cfg(target_os = "android")]
    {
        android_impl::compute_smoke()
    }
    #[cfg(not(target_os = "android"))]
    {
        let start = std::time::Instant::now();
        let spirv = build_square_plus_one_spirv();
        let spirv_ok = validate_spirv_structure(&spirv).is_ok();
        let input: Vec<f32> = (0..16).map(|i| i as f32 * 0.5).collect();
        let out = cpu_square_plus_one(&input);
        let sample_ok = spirv_ok
            && out
                .iter()
                .zip(input.iter())
                .all(|(y, x)| (*y - (x * x + 1.0)).abs() < 1e-5);
        ComputeSmokeResult {
            ok: sample_ok,
            backend: "host-cpu-reference".into(),
            device: "host".into(),
            sample_ok,
            ahb_egl_ok: false,
            ahb_vulkan_ok: false,
            elapsed_us: start.elapsed().as_micros() as u64,
            error: None,
        }
    }
}

#[cfg(target_os = "android")]
pub use android_impl::AndroidGpuRenderer;

#[cfg(not(target_os = "android"))]
pub struct AndroidGpuRenderer;

#[cfg(not(target_os = "android"))]
impl Default for AndroidGpuRenderer {
    fn default() -> Self {
        Self
    }
}

#[cfg(not(target_os = "android"))]
impl AndroidGpuRenderer {
    pub fn new() -> Self {
        Self
    }
}

/// Process-wide GPU runtime shared by the embedded Wayland compositor and any
/// other Android subsystem needing EGL/GLES3 surface rendering or Vulkan compute.
pub struct GpuRuntime {
    renderer: std::sync::Mutex<AndroidGpuRenderer>,
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    compute_lock: std::sync::Mutex<()>,
    force_cpu: std::sync::atomic::AtomicBool,
    cached_caps: std::sync::Mutex<Option<GpuCapabilities>>,
}

static GPU_RUNTIME: std::sync::OnceLock<GpuRuntime> = std::sync::OnceLock::new();

pub fn runtime() -> &'static GpuRuntime {
    GPU_RUNTIME.get_or_init(|| GpuRuntime {
        renderer: std::sync::Mutex::new(AndroidGpuRenderer::new()),
        compute_lock: std::sync::Mutex::new(()),
        force_cpu: std::sync::atomic::AtomicBool::new(false),
        cached_caps: std::sync::Mutex::new(None),
    })
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn f32_slice_to_le_bytes(input: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(input.len() * 4);
    for &v in input {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    bytes
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn le_bytes_to_f32_vec(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

impl GpuRuntime {
    pub fn set_force_cpu(&self, force: bool) {
        self.force_cpu
            .store(force, std::sync::atomic::Ordering::SeqCst);
        if let Ok(mut renderer) = self.renderer.lock() {
            if force {
                #[cfg(target_os = "android")]
                {
                    renderer.release_all_surfaces();
                    renderer.set_gpu_disabled_reason(Some("forced CPU rendering mode".into()));
                }
                #[cfg(not(target_os = "android"))]
                let _ = &mut renderer;
            } else {
                #[cfg(target_os = "android")]
                renderer.clear_gpu_fallback();
            }
        }
        if let Ok(mut guard) = self.cached_caps.lock() {
            *guard = None;
        }
    }

    pub fn is_force_cpu(&self) -> bool {
        self.force_cpu.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn is_cpu_fallback_active(&self) -> bool {
        if self.is_force_cpu() {
            return true;
        }
        #[cfg(target_os = "android")]
        {
            if let Ok(renderer) = self.renderer.lock() {
                if renderer.gpu_disabled_reason().is_some() {
                    return true;
                }
            }
            false
        }
        #[cfg(not(target_os = "android"))]
        {
            true
        }
    }

    pub fn capabilities(&self) -> GpuCapabilities {
        if let Ok(guard) = self.cached_caps.lock() {
            if let Some(ref caps) = *guard {
                let mut out = caps.clone();
                out.force_cpu = self.is_force_cpu();
                out.cpu_fallback_active = self.is_cpu_fallback_active();
                return out;
            }
        }
        self.refresh_capabilities()
    }

    pub fn refresh_capabilities(&self) -> GpuCapabilities {
        let forced = self.is_force_cpu();
        #[cfg(target_os = "android")]
        let mut caps = {
            let mut caps = GpuCapabilities::default();
            if let Ok(mut renderer) = self.renderer.lock() {
                renderer.fill_egl_capabilities(&mut caps);
                if forced {
                    caps.cpu_fallback_active = true;
                    caps.cpu_fallback_reason = Some("forced CPU rendering mode".into());
                } else if let Some(reason) = renderer.gpu_disabled_reason() {
                    caps.cpu_fallback_active = true;
                    caps.cpu_fallback_reason = Some(reason.to_string());
                } else if caps.probe_error.is_some() || caps.egl_renderer.is_empty() {
                    caps.cpu_fallback_active = true;
                    caps.cpu_fallback_reason = caps.probe_error.clone();
                }
            }
            android_impl::fill_vulkan_capabilities(&mut caps);
            caps.force_cpu = forced;
            caps
        };
        #[cfg(not(target_os = "android"))]
        let mut caps = probe_capabilities();
        caps.force_cpu = forced;
        #[cfg(not(target_os = "android"))]
        {
            caps.cpu_fallback_active = true;
            caps.cpu_fallback_reason = if forced {
                Some("forced CPU rendering mode".into())
            } else {
                caps.probe_error.clone()
            };
        }

        if let Ok(mut guard) = self.cached_caps.lock() {
            *guard = Some(caps.clone());
        }
        caps
    }

    pub fn compute_smoke(&self) -> ComputeSmokeResult {
        #[cfg(target_os = "android")]
        {
            let start = std::time::Instant::now();
            let (ahb_egl_ok, fallback_device) = match self.renderer.lock() {
                Ok(mut renderer) => (
                    renderer.ahb_egl_smoke().unwrap_or(false),
                    renderer.egl_renderer_name().to_string(),
                ),
                Err(_) => (false, String::new()),
            };
            let spirv = build_square_plus_one_spirv();
            let input: Vec<f32> = (0..16).map(|i| i as f32 * 0.5).collect();
            let input_bytes = f32_slice_to_le_bytes(&input);
            let _guard = self.compute_lock.lock();
            match unsafe {
                android_impl::run_vulkan_compute_bytes(
                    &spirv,
                    &input_bytes,
                    input_bytes.len(),
                    [((input.len() as u32) + 63) / 64, 1, 1],
                    true,
                )
            } {
                Ok((device, out_bytes, ahb_vulkan_ok)) => {
                    let output = le_bytes_to_f32_vec(&out_bytes);
                    let sample_ok = output.len() == input.len()
                        && output
                            .iter()
                            .zip(input.iter())
                            .all(|(y, x)| (*y - (x * x + 1.0)).abs() < 1e-4);
                    ComputeSmokeResult {
                        ok: sample_ok,
                        backend: "vulkan-compute".into(),
                        device,
                        sample_ok,
                        ahb_egl_ok,
                        ahb_vulkan_ok,
                        elapsed_us: start.elapsed().as_micros() as u64,
                        error: if sample_ok {
                            None
                        } else {
                            Some("Vulkan compute output mismatch".into())
                        },
                    }
                }
                Err(e) => {
                    let out = cpu_square_plus_one(&input);
                    let sample_ok = out
                        .iter()
                        .zip(input.iter())
                        .all(|(y, x)| (*y - (x * x + 1.0)).abs() < 1e-5);
                    ComputeSmokeResult {
                        ok: false,
                        backend: "cpu-fallback".into(),
                        device: fallback_device,
                        sample_ok,
                        ahb_egl_ok,
                        ahb_vulkan_ok: false,
                        elapsed_us: start.elapsed().as_micros() as u64,
                        error: Some(e),
                    }
                }
            }
        }
        #[cfg(not(target_os = "android"))]
        {
            compute_smoke()
        }
    }

    pub fn dispatch_f32_kernel(
        &self,
        kernel: &str,
        input: &[f32],
    ) -> (ComputeDispatchResult, Vec<f32>) {
        let start = std::time::Instant::now();
        let (spirv, cpu_fn, local_size): (Vec<u32>, fn(&[f32]) -> Vec<f32>, u32) = match kernel {
            "square_plus_one" => (build_square_plus_one_spirv(), cpu_square_plus_one, 64),
            "relu" => (build_relu_spirv(), cpu_relu, 1),
            other => {
                return (
                    ComputeDispatchResult {
                        ok: false,
                        used_gpu: false,
                        backend: "none".into(),
                        device: String::new(),
                        elapsed_us: start.elapsed().as_micros() as u64,
                        output_len: 0,
                        error: Some(format!("unknown GPU compute kernel: {other}")),
                    },
                    Vec::new(),
                );
            }
        };

        if input.is_empty() {
            return (
                ComputeDispatchResult {
                    ok: true,
                    used_gpu: false,
                    backend: "empty".into(),
                    device: String::new(),
                    elapsed_us: start.elapsed().as_micros() as u64,
                    output_len: 0,
                    error: None,
                },
                Vec::new(),
            );
        }

        #[cfg(target_os = "android")]
        {
            if self.is_force_cpu() {
                let out = cpu_fn(input);
                return (
                    ComputeDispatchResult {
                        ok: true,
                        used_gpu: false,
                        backend: "cpu-fallback".into(),
                        device: "cpu".into(),
                        elapsed_us: start.elapsed().as_micros() as u64,
                        output_len: out.len() * 4,
                        error: Some("forced CPU mode".into()),
                    },
                    out,
                );
            }
            let input_bytes = f32_slice_to_le_bytes(input);
            let gx = ((input.len() as u32) + local_size - 1) / local_size;
            let _guard = self.compute_lock.lock();
            match unsafe {
                android_impl::run_vulkan_compute_bytes(
                    &spirv,
                    &input_bytes,
                    input_bytes.len(),
                    [gx.max(1), 1, 1],
                    false,
                )
            } {
                Ok((device, out_bytes, _)) => {
                    let output = le_bytes_to_f32_vec(&out_bytes);
                    return (
                        ComputeDispatchResult {
                            ok: output.len() == input.len(),
                            used_gpu: true,
                            backend: "vulkan-compute".into(),
                            device,
                            elapsed_us: start.elapsed().as_micros() as u64,
                            output_len: output.len() * 4,
                            error: None,
                        },
                        output,
                    );
                }
                Err(e) => {
                    let out = cpu_fn(input);
                    return (
                        ComputeDispatchResult {
                            ok: true,
                            used_gpu: false,
                            backend: "cpu-fallback".into(),
                            device: "cpu".into(),
                            elapsed_us: start.elapsed().as_micros() as u64,
                            output_len: out.len() * 4,
                            error: Some(e),
                        },
                        out,
                    );
                }
            }
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = (&spirv, local_size);
            let out = cpu_fn(input);
            (
                ComputeDispatchResult {
                    ok: true,
                    used_gpu: false,
                    backend: "cpu-fallback".into(),
                    device: "host".into(),
                    elapsed_us: start.elapsed().as_micros() as u64,
                    output_len: out.len() * 4,
                    error: None,
                },
                out,
            )
        }
    }

    pub fn dispatch_spirv(
        &self,
        spirv_bytes: &[u8],
        input_bytes: &[u8],
        output_len: usize,
        workgroups: [u32; 3],
    ) -> (ComputeDispatchResult, Vec<u8>) {
        let start = std::time::Instant::now();
        if spirv_bytes.is_empty() || spirv_bytes.len() % 4 != 0 {
            return (
                ComputeDispatchResult {
                    ok: false,
                    used_gpu: false,
                    backend: "invalid-spirv".into(),
                    device: String::new(),
                    elapsed_us: start.elapsed().as_micros() as u64,
                    output_len: 0,
                    error: Some("SPIR-V bytecode length must be a non-empty multiple of 4".into()),
                },
                Vec::new(),
            );
        }
        let words: Vec<u32> = spirv_bytes
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        if let Err(err) = validate_spirv_structure(&words) {
            return (
                ComputeDispatchResult {
                    ok: false,
                    used_gpu: false,
                    backend: "invalid-spirv".into(),
                    device: String::new(),
                    elapsed_us: start.elapsed().as_micros() as u64,
                    output_len: 0,
                    error: Some(err),
                },
                Vec::new(),
            );
        }
        const MAX_COMPUTE_OUT_BYTES: usize = 64 * 1024 * 1024;
        if output_len > MAX_COMPUTE_OUT_BYTES {
            return (
                ComputeDispatchResult {
                    ok: false,
                    used_gpu: false,
                    backend: "invalid-args".into(),
                    device: String::new(),
                    elapsed_us: start.elapsed().as_micros() as u64,
                    output_len: 0,
                    error: Some("output_len exceeds 64 MiB limit".into()),
                },
                Vec::new(),
            );
        }
        if workgroups[0] == 0 || workgroups[1] == 0 || workgroups[2] == 0 {
            return (
                ComputeDispatchResult {
                    ok: false,
                    used_gpu: false,
                    backend: "invalid-args".into(),
                    device: String::new(),
                    elapsed_us: start.elapsed().as_micros() as u64,
                    output_len: 0,
                    error: Some("workgroup dimensions must all be >= 1".into()),
                },
                Vec::new(),
            );
        }

        #[cfg(target_os = "android")]
        {
            if self.is_force_cpu() {
                return (
                    ComputeDispatchResult {
                        ok: false,
                        used_gpu: false,
                        backend: "cpu-forced".into(),
                        device: String::new(),
                        elapsed_us: start.elapsed().as_micros() as u64,
                        output_len: 0,
                        error: Some("GPU disabled (CPU fallback mode active)".into()),
                    },
                    Vec::new(),
                );
            }
            let _guard = self.compute_lock.lock();
            match unsafe {
                android_impl::run_vulkan_compute_bytes(
                    &words,
                    input_bytes,
                    output_len,
                    workgroups,
                    false,
                )
            } {
                Ok((device, out_bytes, _)) => (
                    ComputeDispatchResult {
                        ok: true,
                        used_gpu: true,
                        backend: "vulkan-compute".into(),
                        device,
                        elapsed_us: start.elapsed().as_micros() as u64,
                        output_len: out_bytes.len(),
                        error: None,
                    },
                    out_bytes,
                ),
                Err(e) => (
                    ComputeDispatchResult {
                        ok: false,
                        used_gpu: false,
                        backend: "vulkan-error".into(),
                        device: String::new(),
                        elapsed_us: start.elapsed().as_micros() as u64,
                        output_len: 0,
                        error: Some(e),
                    },
                    Vec::new(),
                ),
            }
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = input_bytes;
            (
                ComputeDispatchResult {
                    ok: false,
                    used_gpu: false,
                    backend: "unavailable".into(),
                    device: "host".into(),
                    elapsed_us: start.elapsed().as_micros() as u64,
                    output_len: 0,
                    error: Some("Vulkan compute dispatch requires Android Vulkan runtime".into()),
                },
                Vec::new(),
            )
        }
    }

    pub fn release_surface(&self, surface_key: &str) {
        #[cfg(target_os = "android")]
        {
            if let Ok(mut renderer) = self.renderer.lock() {
                renderer.release_surface(surface_key);
            }
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = surface_key;
        }
    }

    #[cfg(target_os = "android")]
    pub unsafe fn render_shm(
        &self,
        surface_key: &str,
        window: *mut android_impl::ANativeWindow,
        frame: ShmFrameRef<'_>,
        crop: [f32; 4],
    ) -> Result<RenderResult, String> {
        let mut renderer = self
            .renderer
            .lock()
            .map_err(|_| "GPU renderer mutex poisoned".to_string())?;
        unsafe { renderer.render_shm(surface_key, window, frame, crop) }
    }

    #[cfg(target_os = "android")]
    pub unsafe fn render_dmabuf(
        &self,
        surface_key: &str,
        window: *mut android_impl::ANativeWindow,
        dmabuf: &DmabufDescriptor,
        crop: [f32; 4],
    ) -> Result<RenderResult, String> {
        let mut renderer = self
            .renderer
            .lock()
            .map_err(|_| "GPU renderer mutex poisoned".to_string())?;
        unsafe { renderer.render_dmabuf(surface_key, window, dmabuf, crop) }
    }

    #[cfg(target_os = "android")]
    pub unsafe fn render_placeholder(
        &self,
        surface_key: &str,
        window: *mut android_impl::ANativeWindow,
    ) -> Result<RenderResult, String> {
        let mut renderer = self
            .renderer
            .lock()
            .map_err(|_| "GPU renderer mutex poisoned".to_string())?;
        unsafe { renderer.render_placeholder(surface_key, window) }
    }
}

#[cfg(target_os = "android")]
pub(crate) mod android_impl {
    use super::*;
    use std::collections::HashMap;
    use std::ffi::{c_char, c_void, CStr};
    use std::os::fd::AsRawFd;
    use std::ptr;

    type EGLDisplay = *mut c_void;
    type EGLConfig = *mut c_void;
    type EGLContext = *mut c_void;
    type EGLSurface = *mut c_void;
    type EGLImageKHR = *mut c_void;
    type EGLClientBuffer = *mut c_void;
    type EGLNativeWindowType = *mut c_void;
    type EGLBoolean = u32;
    type EGLint = i32;
    type EGLenum = u32;

    const EGL_FALSE: EGLBoolean = 0;
    const EGL_TRUE: EGLBoolean = 1;
    const EGL_NONE: EGLint = 0x3038;
    const EGL_NO_DISPLAY: EGLDisplay = ptr::null_mut();
    const EGL_NO_CONTEXT: EGLContext = ptr::null_mut();
    const EGL_NO_SURFACE: EGLSurface = ptr::null_mut();
    const EGL_NO_IMAGE_KHR: EGLImageKHR = ptr::null_mut();
    const EGL_DEFAULT_DISPLAY: *mut c_void = ptr::null_mut();

    const EGL_RED_SIZE: EGLint = 0x3024;
    const EGL_GREEN_SIZE: EGLint = 0x3023;
    const EGL_BLUE_SIZE: EGLint = 0x3022;
    const EGL_ALPHA_SIZE: EGLint = 0x3021;
    const EGL_SURFACE_TYPE: EGLint = 0x3033;
    const EGL_WINDOW_BIT: EGLint = 0x0004;
    const EGL_PBUFFER_BIT: EGLint = 0x0001;
    const EGL_RENDERABLE_TYPE: EGLint = 0x3040;
    const EGL_OPENGL_ES2_BIT: EGLint = 0x0004;
    const EGL_OPENGL_ES3_BIT_KHR: EGLint = 0x0040;
    const EGL_WIDTH: EGLint = 0x3057;
    const EGL_HEIGHT: EGLint = 0x3056;
    const EGL_CONTEXT_CLIENT_VERSION: EGLint = 0x3098;
    const EGL_VENDOR: EGLint = 0x3053;
    const EGL_VERSION: EGLint = 0x3054;
    const EGL_EXTENSIONS: EGLint = 0x3055;

    const EGL_NATIVE_BUFFER_ANDROID: EGLenum = 0x3140;
    const EGL_IMAGE_PRESERVED_KHR: EGLint = 0x30D2;
    const EGL_LINUX_DMA_BUF_EXT: EGLenum = 0x3270;
    const EGL_LINUX_DRM_FOURCC_EXT: EGLint = 0x3271;
    const EGL_DMA_BUF_PLANE0_FD_EXT: EGLint = 0x3272;
    const EGL_DMA_BUF_PLANE0_OFFSET_EXT: EGLint = 0x3273;
    const EGL_DMA_BUF_PLANE0_PITCH_EXT: EGLint = 0x3274;
    const EGL_DMA_BUF_PLANE1_FD_EXT: EGLint = 0x3275;
    const EGL_DMA_BUF_PLANE1_OFFSET_EXT: EGLint = 0x3276;
    const EGL_DMA_BUF_PLANE1_PITCH_EXT: EGLint = 0x3277;
    const EGL_DMA_BUF_PLANE2_FD_EXT: EGLint = 0x3278;
    const EGL_DMA_BUF_PLANE2_OFFSET_EXT: EGLint = 0x3279;
    const EGL_DMA_BUF_PLANE2_PITCH_EXT: EGLint = 0x327A;
    const EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT: EGLint = 0x3443;
    const EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT: EGLint = 0x3444;
    const EGL_DMA_BUF_PLANE1_MODIFIER_LO_EXT: EGLint = 0x3445;
    const EGL_DMA_BUF_PLANE1_MODIFIER_HI_EXT: EGLint = 0x3446;
    const EGL_DMA_BUF_PLANE2_MODIFIER_LO_EXT: EGLint = 0x3447;
    const EGL_DMA_BUF_PLANE2_MODIFIER_HI_EXT: EGLint = 0x3448;

    const GL_NO_ERROR: u32 = 0;
    const GL_COLOR_BUFFER_BIT: u32 = 0x0000_4000;
    const GL_TRIANGLE_STRIP: u32 = 0x0005;
    const GL_FLOAT: u32 = 0x1406;
    const GL_UNSIGNED_BYTE: u32 = 0x1401;
    const GL_RGBA: u32 = 0x1908;
    const GL_TEXTURE_2D: u32 = 0x0DE1;
    const GL_TEXTURE_EXTERNAL_OES: u32 = 0x8D65;
    const GL_TEXTURE_MAG_FILTER: u32 = 0x2800;
    const GL_TEXTURE_MIN_FILTER: u32 = 0x2801;
    const GL_TEXTURE_WRAP_S: u32 = 0x2802;
    const GL_TEXTURE_WRAP_T: u32 = 0x2803;
    const GL_LINEAR: i32 = 0x2601;
    const GL_CLAMP_TO_EDGE: i32 = 0x812F;
    const GL_TEXTURE0: u32 = 0x84C0;
    const GL_FRAGMENT_SHADER: u32 = 0x8B30;
    const GL_VERTEX_SHADER: u32 = 0x8B31;
    const GL_COMPILE_STATUS: u32 = 0x8B81;
    const GL_LINK_STATUS: u32 = 0x8B82;
    const GL_RENDERER: u32 = 0x1F01;
    const GL_VERSION: u32 = 0x1F02;
    const GL_EXTENSIONS: u32 = 0x1F03;
    const GL_UNPACK_ALIGNMENT: u32 = 0x0CF5;
    const GL_UNPACK_ROW_LENGTH: u32 = 0x0CF2;

    #[repr(C)]
    pub struct ANativeWindow {
        _private: [u8; 0],
    }

    #[repr(C)]
    struct AHardwareBuffer {
        _private: [u8; 0],
    }

    #[repr(C)]
    struct AHardwareBufferDesc {
        width: u32,
        height: u32,
        layers: u32,
        format: u32,
        usage: u64,
        stride: u32,
        rfu0: u32,
        rfu1: u64,
    }

    const AHARDWAREBUFFER_FORMAT_R8G8B8A8_UNORM: u32 = 1;
    const AHARDWAREBUFFER_USAGE_CPU_WRITE_OFTEN: u64 = 3 << 4;
    const AHARDWAREBUFFER_USAGE_GPU_SAMPLED_IMAGE: u64 = 1 << 8;
    const AHARDWAREBUFFER_USAGE_GPU_COLOR_OUTPUT: u64 = 1 << 9;

    #[link(name = "EGL")]
    unsafe extern "C" {
        fn eglGetDisplay(display_id: *mut c_void) -> EGLDisplay;
        fn eglInitialize(dpy: EGLDisplay, major: *mut EGLint, minor: *mut EGLint) -> EGLBoolean;
        fn eglTerminate(dpy: EGLDisplay) -> EGLBoolean;
        fn eglChooseConfig(
            dpy: EGLDisplay,
            attrib_list: *const EGLint,
            configs: *mut EGLConfig,
            config_size: EGLint,
            num_config: *mut EGLint,
        ) -> EGLBoolean;
        fn eglCreateContext(
            dpy: EGLDisplay,
            config: EGLConfig,
            share_context: EGLContext,
            attrib_list: *const EGLint,
        ) -> EGLContext;
        fn eglDestroyContext(dpy: EGLDisplay, ctx: EGLContext) -> EGLBoolean;
        fn eglCreateWindowSurface(
            dpy: EGLDisplay,
            config: EGLConfig,
            win: EGLNativeWindowType,
            attrib_list: *const EGLint,
        ) -> EGLSurface;
        fn eglCreatePbufferSurface(
            dpy: EGLDisplay,
            config: EGLConfig,
            attrib_list: *const EGLint,
        ) -> EGLSurface;
        fn eglDestroySurface(dpy: EGLDisplay, surface: EGLSurface) -> EGLBoolean;
        fn eglMakeCurrent(
            dpy: EGLDisplay,
            draw: EGLSurface,
            read: EGLSurface,
            ctx: EGLContext,
        ) -> EGLBoolean;
        fn eglSwapBuffers(dpy: EGLDisplay, surface: EGLSurface) -> EGLBoolean;
        fn eglQueryString(dpy: EGLDisplay, name: EGLint) -> *const c_char;
        fn eglGetProcAddress(procname: *const c_char) -> *const c_void;
        fn eglGetError() -> EGLint;
    }

    #[link(name = "GLESv3")]
    unsafe extern "C" {
        fn glGetString(name: u32) -> *const u8;
        fn glGetError() -> u32;
        fn glViewport(x: i32, y: i32, width: i32, height: i32);
        fn glClearColor(r: f32, g: f32, b: f32, a: f32);
        fn glClear(mask: u32);
        fn glGenTextures(n: i32, textures: *mut u32);
        fn glDeleteTextures(n: i32, textures: *const u32);
        fn glActiveTexture(texture: u32);
        fn glBindTexture(target: u32, texture: u32);
        fn glTexParameteri(target: u32, pname: u32, param: i32);
        fn glPixelStorei(pname: u32, param: i32);
        fn glTexImage2D(
            target: u32,
            level: i32,
            internalformat: i32,
            width: i32,
            height: i32,
            border: i32,
            format: u32,
            type_: u32,
            pixels: *const c_void,
        );
        fn glCreateShader(type_: u32) -> u32;
        fn glShaderSource(
            shader: u32,
            count: i32,
            string: *const *const c_char,
            length: *const i32,
        );
        fn glCompileShader(shader: u32);
        fn glGetShaderiv(shader: u32, pname: u32, params: *mut i32);
        fn glDeleteShader(shader: u32);
        fn glCreateProgram() -> u32;
        fn glAttachShader(program: u32, shader: u32);
        fn glLinkProgram(program: u32);
        fn glGetProgramiv(program: u32, pname: u32, params: *mut i32);
        fn glDeleteProgram(program: u32);
        fn glUseProgram(program: u32);
        fn glGetAttribLocation(program: u32, name: *const c_char) -> i32;
        fn glGetUniformLocation(program: u32, name: *const c_char) -> i32;
        fn glEnableVertexAttribArray(index: u32);
        fn glDisableVertexAttribArray(index: u32);
        fn glVertexAttribPointer(
            index: u32,
            size: i32,
            type_: u32,
            normalized: u8,
            stride: i32,
            pointer: *const c_void,
        );
        fn glUniform1i(location: i32, v0: i32);
        fn glUniform1f(location: i32, v0: f32);
        fn glDrawArrays(mode: u32, first: i32, count: i32);
    }

    const WINDOW_FORMAT_RGBA_8888: i32 = 1;

    #[repr(C)]
    pub struct ANativeWindowBuffer {
        pub width: i32,
        pub height: i32,
        pub stride: i32,
        pub format: i32,
        pub bits: *mut c_void,
        pub reserved: [u32; 6],
    }

    #[link(name = "android")]
    unsafe extern "C" {
        pub fn ANativeWindow_fromSurface(
            env: *mut jni::sys::JNIEnv,
            surface: jni::sys::jobject,
        ) -> *mut ANativeWindow;
        pub fn ANativeWindow_acquire(window: *mut ANativeWindow);
        pub fn ANativeWindow_release(window: *mut ANativeWindow);
        pub fn ANativeWindow_getWidth(window: *mut ANativeWindow) -> i32;
        pub fn ANativeWindow_getHeight(window: *mut ANativeWindow) -> i32;
        pub fn ANativeWindow_setBuffersGeometry(
            window: *mut ANativeWindow,
            width: i32,
            height: i32,
            format: i32,
        ) -> i32;
        pub fn ANativeWindow_lock(
            window: *mut ANativeWindow,
            out_buffer: *mut ANativeWindowBuffer,
            in_out_dirty_bounds: *mut c_void,
        ) -> i32;
        pub fn ANativeWindow_unlockAndPost(window: *mut ANativeWindow) -> i32;
        fn AHardwareBuffer_allocate(
            desc: *const AHardwareBufferDesc,
            out_buffer: *mut *mut AHardwareBuffer,
        ) -> i32;
        fn AHardwareBuffer_release(buffer: *mut AHardwareBuffer);
    }

    type FnEglCreateImageKHR = unsafe extern "C" fn(
        dpy: EGLDisplay,
        ctx: EGLContext,
        target: EGLenum,
        buffer: EGLClientBuffer,
        attrib_list: *const EGLint,
    ) -> EGLImageKHR;
    type FnEglDestroyImageKHR =
        unsafe extern "C" fn(dpy: EGLDisplay, image: EGLImageKHR) -> EGLBoolean;
    type FnGlEGLImageTargetTexture2DOES = unsafe extern "C" fn(target: u32, image: EGLImageKHR);
    type FnEglGetNativeClientBufferANDROID =
        unsafe extern "C" fn(buffer: *const AHardwareBuffer) -> EGLClientBuffer;

    unsafe fn cstr_opt(ptr: *const c_char) -> String {
        if ptr.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(ptr) }
                .to_string_lossy()
                .into_owned()
        }
    }

    unsafe fn glstr_opt(ptr: *const u8) -> String {
        if ptr.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(ptr as *const c_char) }
                .to_string_lossy()
                .into_owned()
        }
    }

    struct QuadProgram {
        program: u32,
        pos_loc: i32,
        uv_loc: i32,
        tex_loc: i32,
        swap_rb_loc: i32,
        opaque_loc: i32,
    }

    impl QuadProgram {
        unsafe fn compile(external: bool) -> Result<Self, String> {
            let vsrc = c"attribute vec2 a_pos;
attribute vec2 a_uv;
varying vec2 v_uv;
void main() {
    gl_Position = vec4(a_pos, 0.0, 1.0);
    v_uv = a_uv;
}
";
            let fsrc = if external {
                c"#extension GL_OES_EGL_image_external : require
precision mediump float;
uniform samplerExternalOES u_tex;
uniform float u_swap_rb;
uniform float u_opaque;
varying vec2 v_uv;
void main() {
    vec4 c = texture2D(u_tex, v_uv);
    vec4 rgb = mix(c, vec4(c.b, c.g, c.r, c.a), u_swap_rb);
    gl_FragColor = vec4(rgb.rgb, mix(rgb.a, 1.0, u_opaque));
}
"
            } else {
                c"precision mediump float;
uniform sampler2D u_tex;
uniform float u_swap_rb;
uniform float u_opaque;
varying vec2 v_uv;
void main() {
    vec4 c = texture2D(u_tex, v_uv);
    vec4 rgb = mix(c, vec4(c.b, c.g, c.r, c.a), u_swap_rb);
    gl_FragColor = vec4(rgb.rgb, mix(rgb.a, 1.0, u_opaque));
}
"
            };
            unsafe {
                let vs = compile_shader(GL_VERTEX_SHADER, vsrc)?;
                let fs = match compile_shader(GL_FRAGMENT_SHADER, fsrc) {
                    Ok(s) => s,
                    Err(e) => {
                        glDeleteShader(vs);
                        return Err(e);
                    }
                };
                let program = glCreateProgram();
                glAttachShader(program, vs);
                glAttachShader(program, fs);
                glLinkProgram(program);
                let mut linked = 0i32;
                glGetProgramiv(program, GL_LINK_STATUS, &mut linked);
                glDeleteShader(vs);
                glDeleteShader(fs);
                if linked == 0 {
                    glDeleteProgram(program);
                    return Err("GLES shader program link failed".into());
                }
                Ok(Self {
                    program,
                    pos_loc: glGetAttribLocation(program, c"a_pos".as_ptr()),
                    uv_loc: glGetAttribLocation(program, c"a_uv".as_ptr()),
                    tex_loc: glGetUniformLocation(program, c"u_tex".as_ptr()),
                    swap_rb_loc: glGetUniformLocation(program, c"u_swap_rb".as_ptr()),
                    opaque_loc: glGetUniformLocation(program, c"u_opaque".as_ptr()),
                })
            }
        }

        unsafe fn draw(
            &self,
            tex_target: u32,
            tex: u32,
            crop: [f32; 4],
            swap_rb: bool,
            opaque: bool,
        ) {
            let [u0, v0, u1, v1] = crop;
            let verts: [f32; 8] = [-1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0, -1.0];
            let uvs: [f32; 8] = [u0, v0, u0, v1, u1, v0, u1, v1];
            unsafe {
                glUseProgram(self.program);
                glActiveTexture(GL_TEXTURE0);
                glBindTexture(tex_target, tex);
                glUniform1i(self.tex_loc, 0);
                if self.swap_rb_loc >= 0 {
                    glUniform1f(self.swap_rb_loc, if swap_rb { 1.0 } else { 0.0 });
                }
                if self.opaque_loc >= 0 {
                    glUniform1f(self.opaque_loc, if opaque { 1.0 } else { 0.0 });
                }
                if self.pos_loc >= 0 {
                    glEnableVertexAttribArray(self.pos_loc as u32);
                    glVertexAttribPointer(
                        self.pos_loc as u32,
                        2,
                        GL_FLOAT,
                        0,
                        0,
                        verts.as_ptr() as *const c_void,
                    );
                }
                if self.uv_loc >= 0 {
                    glEnableVertexAttribArray(self.uv_loc as u32);
                    glVertexAttribPointer(
                        self.uv_loc as u32,
                        2,
                        GL_FLOAT,
                        0,
                        0,
                        uvs.as_ptr() as *const c_void,
                    );
                }
                glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
                if self.pos_loc >= 0 {
                    glDisableVertexAttribArray(self.pos_loc as u32);
                }
                if self.uv_loc >= 0 {
                    glDisableVertexAttribArray(self.uv_loc as u32);
                }
            }
        }
    }

    unsafe fn compile_shader(kind: u32, src: &CStr) -> Result<u32, String> {
        unsafe {
            let shader = glCreateShader(kind);
            let ptr = src.as_ptr();
            glShaderSource(shader, 1, &ptr, ptr::null());
            glCompileShader(shader);
            let mut ok = 0i32;
            glGetShaderiv(shader, GL_COMPILE_STATUS, &mut ok);
            if ok == 0 {
                glDeleteShader(shader);
                return Err(format!("GLES shader compile failed (kind={kind})"));
            }
            Ok(shader)
        }
    }

    struct SurfaceSlot {
        window: *mut ANativeWindow,
        egl_surface: EGLSurface,
    }

    /// Thread-safe (under external Mutex) EGL/GLES3 compositor owning shared EGL context
    /// and per-figure `ANativeWindow` window surfaces.
    pub struct AndroidGpuRenderer {
        display: EGLDisplay,
        config: EGLConfig,
        context: EGLContext,
        pbuffer: EGLSurface,
        is_gles3: bool,
        egl_vendor: String,
        egl_renderer: String,
        egl_version: String,
        gles_version: String,
        egl_exts: String,
        gl_exts: String,
        shm_tex: u32,
        ext_tex: u32,
        prog_2d: Option<QuadProgram>,
        prog_ext: Option<QuadProgram>,
        fn_create_image: Option<FnEglCreateImageKHR>,
        fn_destroy_image: Option<FnEglDestroyImageKHR>,
        fn_image_target_tex: Option<FnGlEGLImageTargetTexture2DOES>,
        fn_get_native_client_buf: Option<FnEglGetNativeClientBufferANDROID>,
        surfaces: HashMap<String, SurfaceSlot>,
        initialized: bool,
        init_error: Option<String>,
        gpu_disabled_reason: Option<String>,
    }

    // All EGL/GLES calls release the context before dropping the external Mutex.
    unsafe impl Send for AndroidGpuRenderer {}

    impl Default for AndroidGpuRenderer {
        fn default() -> Self {
            Self::new()
        }
    }

    impl AndroidGpuRenderer {
        pub fn new() -> Self {
            Self {
                display: EGL_NO_DISPLAY,
                config: ptr::null_mut(),
                context: EGL_NO_CONTEXT,
                pbuffer: EGL_NO_SURFACE,
                is_gles3: false,
                egl_vendor: String::new(),
                egl_renderer: String::new(),
                egl_version: String::new(),
                gles_version: String::new(),
                egl_exts: String::new(),
                gl_exts: String::new(),
                shm_tex: 0,
                ext_tex: 0,
                prog_2d: None,
                prog_ext: None,
                fn_create_image: None,
                fn_destroy_image: None,
                fn_image_target_tex: None,
                fn_get_native_client_buf: None,
                surfaces: HashMap::new(),
                initialized: false,
                init_error: None,
                gpu_disabled_reason: None,
            }
        }

        pub fn gpu_disabled_reason(&self) -> Option<&str> {
            self.gpu_disabled_reason.as_deref()
        }

        pub fn set_gpu_disabled_reason(&mut self, reason: Option<String>) {
            self.gpu_disabled_reason = reason;
        }

        pub fn clear_gpu_fallback(&mut self) {
            self.gpu_disabled_reason = None;
            if self.init_error.is_some() {
                self.initialized = false;
                self.init_error = None;
            }
        }

        pub fn release_all_surfaces(&mut self) {
            let keys: Vec<String> = self.surfaces.keys().cloned().collect();
            for k in keys {
                self.release_surface(&k);
            }
        }

        fn ensure_init(&mut self) -> Result<(), String> {
            if self.initialized {
                return match &self.init_error {
                    Some(e) => Err(e.clone()),
                    None => Ok(()),
                };
            }
            self.initialized = true;
            match unsafe { self.init_egl() } {
                Ok(()) => Ok(()),
                Err(e) => {
                    self.init_error = Some(e.clone());
                    Err(e)
                }
            }
        }

        unsafe fn init_egl(&mut self) -> Result<(), String> {
            unsafe {
                let dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);
                if dpy == EGL_NO_DISPLAY {
                    return Err("eglGetDisplay returned EGL_NO_DISPLAY".into());
                }
                let mut maj = 0;
                let mut min = 0;
                if eglInitialize(dpy, &mut maj, &mut min) == EGL_FALSE {
                    return Err(format!("eglInitialize failed (0x{:x})", eglGetError()));
                }
                self.display = dpy;

                let mut cfg: EGLConfig = ptr::null_mut();
                let mut ncfg = 0;
                let attrs_es3 = [
                    EGL_RENDERABLE_TYPE,
                    EGL_OPENGL_ES3_BIT_KHR,
                    EGL_SURFACE_TYPE,
                    EGL_WINDOW_BIT | EGL_PBUFFER_BIT,
                    EGL_RED_SIZE,
                    8,
                    EGL_GREEN_SIZE,
                    8,
                    EGL_BLUE_SIZE,
                    8,
                    EGL_ALPHA_SIZE,
                    8,
                    EGL_NONE,
                ];
                let attrs_es2 = [
                    EGL_RENDERABLE_TYPE,
                    EGL_OPENGL_ES2_BIT,
                    EGL_SURFACE_TYPE,
                    EGL_WINDOW_BIT | EGL_PBUFFER_BIT,
                    EGL_RED_SIZE,
                    8,
                    EGL_GREEN_SIZE,
                    8,
                    EGL_BLUE_SIZE,
                    8,
                    EGL_ALPHA_SIZE,
                    8,
                    EGL_NONE,
                ];
                let mut use_es3 = eglChooseConfig(dpy, attrs_es3.as_ptr(), &mut cfg, 1, &mut ncfg)
                    == EGL_TRUE
                    && ncfg > 0;
                if !use_es3
                    && (eglChooseConfig(dpy, attrs_es2.as_ptr(), &mut cfg, 1, &mut ncfg)
                        == EGL_FALSE
                        || ncfg == 0)
                {
                    return Err(format!("eglChooseConfig failed (0x{:x})", eglGetError()));
                }
                self.config = cfg;

                let ctx_es3 = [EGL_CONTEXT_CLIENT_VERSION, 3, EGL_NONE];
                let ctx_es2 = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
                let mut ctx = if use_es3 {
                    eglCreateContext(dpy, cfg, EGL_NO_CONTEXT, ctx_es3.as_ptr())
                } else {
                    EGL_NO_CONTEXT
                };
                if ctx == EGL_NO_CONTEXT {
                    use_es3 = false;
                    ctx = eglCreateContext(dpy, cfg, EGL_NO_CONTEXT, ctx_es2.as_ptr());
                }
                if ctx == EGL_NO_CONTEXT {
                    return Err(format!("eglCreateContext failed (0x{:x})", eglGetError()));
                }
                self.context = ctx;
                self.is_gles3 = use_es3;

                let pb_attrs = [EGL_WIDTH, 16, EGL_HEIGHT, 16, EGL_NONE];
                let pbuffer = eglCreatePbufferSurface(dpy, cfg, pb_attrs.as_ptr());
                if pbuffer == EGL_NO_SURFACE {
                    return Err(format!(
                        "eglCreatePbufferSurface failed (0x{:x})",
                        eglGetError()
                    ));
                }
                self.pbuffer = pbuffer;

                if eglMakeCurrent(dpy, pbuffer, pbuffer, ctx) == EGL_FALSE {
                    return Err(format!(
                        "eglMakeCurrent(pbuffer) failed (0x{:x})",
                        eglGetError()
                    ));
                }

                self.egl_vendor = cstr_opt(eglQueryString(dpy, EGL_VENDOR));
                self.egl_version = cstr_opt(eglQueryString(dpy, EGL_VERSION));
                self.egl_exts = cstr_opt(eglQueryString(dpy, EGL_EXTENSIONS));
                self.egl_renderer = glstr_opt(glGetString(GL_RENDERER));
                self.gles_version = glstr_opt(glGetString(GL_VERSION));
                self.gl_exts = glstr_opt(glGetString(GL_EXTENSIONS));

                let p_create = eglGetProcAddress(c"eglCreateImageKHR".as_ptr());
                let p_destroy = eglGetProcAddress(c"eglDestroyImageKHR".as_ptr());
                let p_target = eglGetProcAddress(c"glEGLImageTargetTexture2DOES".as_ptr());
                let p_native_buf = eglGetProcAddress(c"eglGetNativeClientBufferANDROID".as_ptr());
                if !p_create.is_null() {
                    self.fn_create_image = Some(std::mem::transmute(p_create));
                }
                if !p_destroy.is_null() {
                    self.fn_destroy_image = Some(std::mem::transmute(p_destroy));
                }
                if !p_target.is_null() {
                    self.fn_image_target_tex = Some(std::mem::transmute(p_target));
                }
                if !p_native_buf.is_null() {
                    self.fn_get_native_client_buf = Some(std::mem::transmute(p_native_buf));
                }

                let mut texs = [0u32; 2];
                glGenTextures(2, texs.as_mut_ptr());
                self.shm_tex = texs[0];
                self.ext_tex = texs[1];

                glBindTexture(GL_TEXTURE_2D, self.shm_tex);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);

                self.prog_2d = Some(QuadProgram::compile(false)?);
                if has_ext(&self.gl_exts, "GL_OES_EGL_image_external") {
                    glBindTexture(GL_TEXTURE_EXTERNAL_OES, self.ext_tex);
                    glTexParameteri(GL_TEXTURE_EXTERNAL_OES, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
                    glTexParameteri(GL_TEXTURE_EXTERNAL_OES, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
                    glTexParameteri(GL_TEXTURE_EXTERNAL_OES, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
                    glTexParameteri(GL_TEXTURE_EXTERNAL_OES, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
                    self.prog_ext = QuadProgram::compile(true).ok();
                }

                eglMakeCurrent(dpy, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
                Ok(())
            }
        }

        pub fn egl_renderer_name(&self) -> &str {
            &self.egl_renderer
        }

        pub fn fill_egl_capabilities(&mut self, caps: &mut GpuCapabilities) {
            if let Err(e) = self.ensure_init() {
                caps.probe_error = Some(e);
                return;
            }
            caps.egl_vendor = self.egl_vendor.clone();
            caps.egl_renderer = self.egl_renderer.clone();
            caps.egl_version = self.egl_version.clone();
            caps.gles_version = self.gles_version.clone();
            caps.has_gles3 = self.is_gles3;
            caps.has_egl_image_base = has_ext(&self.egl_exts, "EGL_KHR_image_base");
            caps.has_egl_android_image_native_buffer =
                has_ext(&self.egl_exts, "EGL_ANDROID_image_native_buffer");
            caps.has_egl_dmabuf_import = has_ext(&self.egl_exts, "EGL_EXT_image_dma_buf_import");
            caps.has_egl_dmabuf_modifiers =
                has_ext(&self.egl_exts, "EGL_EXT_image_dma_buf_import_modifiers");
            caps.has_egl_native_fence_sync =
                has_ext(&self.egl_exts, "EGL_ANDROID_native_fence_sync");
            caps.has_gl_oes_egl_image_external =
                has_ext(&self.gl_exts, "GL_OES_EGL_image_external");
        }

        /// Test zero-copy `AHardwareBuffer` -> `EGLImageKHR` -> `GL_TEXTURE_EXTERNAL_OES` binding.
        pub fn ahb_egl_smoke(&mut self) -> Result<bool, String> {
            self.ensure_init()?;
            let (Some(create_img), Some(destroy_img), Some(target_tex), Some(get_client_buf)) = (
                self.fn_create_image,
                self.fn_destroy_image,
                self.fn_image_target_tex,
                self.fn_get_native_client_buf,
            ) else {
                return Ok(false);
            };
            if self.prog_ext.is_none() {
                return Ok(false);
            }
            unsafe {
                let desc = AHardwareBufferDesc {
                    width: 64,
                    height: 64,
                    layers: 1,
                    format: AHARDWAREBUFFER_FORMAT_R8G8B8A8_UNORM,
                    usage: AHARDWAREBUFFER_USAGE_GPU_SAMPLED_IMAGE
                        | AHARDWAREBUFFER_USAGE_GPU_COLOR_OUTPUT
                        | AHARDWAREBUFFER_USAGE_CPU_WRITE_OFTEN,
                    stride: 0,
                    rfu0: 0,
                    rfu1: 0,
                };
                let mut ahb: *mut AHardwareBuffer = ptr::null_mut();
                if AHardwareBuffer_allocate(&desc, &mut ahb) != 0 || ahb.is_null() {
                    return Err("AHardwareBuffer_allocate failed".into());
                }
                let client_buf = get_client_buf(ahb);
                if client_buf.is_null() {
                    AHardwareBuffer_release(ahb);
                    return Err("eglGetNativeClientBufferANDROID returned null".into());
                }
                if eglMakeCurrent(self.display, self.pbuffer, self.pbuffer, self.context)
                    == EGL_FALSE
                {
                    AHardwareBuffer_release(ahb);
                    return Err("eglMakeCurrent failed for AHB smoke".into());
                }
                let attrs = [EGL_IMAGE_PRESERVED_KHR, EGL_TRUE as i32, EGL_NONE];
                let img = create_img(
                    self.display,
                    EGL_NO_CONTEXT,
                    EGL_NATIVE_BUFFER_ANDROID,
                    client_buf,
                    attrs.as_ptr(),
                );
                let ok = if img != EGL_NO_IMAGE_KHR {
                    glViewport(0, 0, 16, 16);
                    glClearColor(0.08, 0.10, 0.12, 1.0);
                    glClear(GL_COLOR_BUFFER_BIT);
                    glBindTexture(GL_TEXTURE_EXTERNAL_OES, self.ext_tex);
                    target_tex(GL_TEXTURE_EXTERNAL_OES, img);
                    if let Some(ref prog) = self.prog_ext {
                        prog.draw(
                            GL_TEXTURE_EXTERNAL_OES,
                            self.ext_tex,
                            [0.0, 0.0, 1.0, 1.0],
                            false,
                            true,
                        );
                    }
                    let err = glGetError();
                    destroy_img(self.display, img);
                    err == GL_NO_ERROR
                } else {
                    false
                };
                eglMakeCurrent(self.display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
                AHardwareBuffer_release(ahb);
                Ok(ok)
            }
        }

        pub fn release_surface(&mut self, key: &str) {
            if let Some(slot) = self.surfaces.remove(key) {
                unsafe {
                    if self.display != EGL_NO_DISPLAY && slot.egl_surface != EGL_NO_SURFACE {
                        eglDestroySurface(self.display, slot.egl_surface);
                    }
                    if !slot.window.is_null() {
                        ANativeWindow_release(slot.window);
                    }
                }
            }
        }

        unsafe fn ensure_window_surface(
            &mut self,
            key: &str,
            window: *mut ANativeWindow,
        ) -> Result<(EGLSurface, i32, i32), String> {
            self.ensure_init()?;
            if window.is_null() {
                return Err("null ANativeWindow".into());
            }
            let replace = match self.surfaces.get(key) {
                Some(slot) => slot.window != window,
                None => true,
            };
            if replace {
                self.release_surface(key);
                let egl_surface = unsafe {
                    eglCreateWindowSurface(
                        self.display,
                        self.config,
                        window as *mut c_void,
                        ptr::null(),
                    )
                };
                if egl_surface == EGL_NO_SURFACE {
                    return Err(format!("eglCreateWindowSurface failed (0x{:x})", unsafe {
                        eglGetError()
                    }));
                }
                unsafe { ANativeWindow_acquire(window) };
                self.surfaces.insert(
                    key.to_string(),
                    SurfaceSlot {
                        window,
                        egl_surface,
                    },
                );
            }
            let slot = self.surfaces.get(key).expect("inserted above");
            let w = unsafe { ANativeWindow_getWidth(slot.window) }.max(1);
            let h = unsafe { ANativeWindow_getHeight(slot.window) }.max(1);
            Ok((slot.egl_surface, w, h))
        }

        fn active_gl_backend(&self) -> &'static str {
            if self.is_gles3 {
                "egl-gles3"
            } else {
                "egl-gles2"
            }
        }

        unsafe fn cpu_render_shm(
            window: *mut ANativeWindow,
            frame: ShmFrameRef<'_>,
            crop: [f32; 4],
            mode: &'static str,
            reason: String,
        ) -> Result<RenderResult, String> {
            if window.is_null() {
                return Err("null ANativeWindow".into());
            }
            if frame.width == 0 || frame.height == 0 || frame.stride < frame.width * 4 {
                return Err("invalid wl_shm frame dimensions".into());
            }
            let needed = frame.stride as usize * frame.height as usize;
            if frame.pixels.len() < needed {
                return Err("truncated wl_shm pixel buffer".into());
            }
            unsafe {
                let mut w = ANativeWindow_getWidth(window);
                let mut h = ANativeWindow_getHeight(window);
                if w <= 0 || h <= 0 {
                    w = frame.width as i32;
                    h = frame.height as i32;
                }
                let rc_geom =
                    ANativeWindow_setBuffersGeometry(window, w, h, WINDOW_FORMAT_RGBA_8888);
                if rc_geom != 0 {
                    return Err(format!(
                        "ANativeWindow_setBuffersGeometry failed ({rc_geom})"
                    ));
                }
                let mut buf = ANativeWindowBuffer {
                    width: 0,
                    height: 0,
                    stride: 0,
                    format: 0,
                    bits: ptr::null_mut(),
                    reserved: [0; 6],
                };
                let rc_lock = ANativeWindow_lock(window, &mut buf, ptr::null_mut());
                if rc_lock != 0 || buf.bits.is_null() {
                    return Err(format!("ANativeWindow_lock failed ({rc_lock})"));
                }
                let bw = buf.width.max(0) as usize;
                let bh = buf.height.max(0) as usize;
                let bstride = buf.stride.max(0) as usize;
                let blit_res = if bw == 0 || bh == 0 || bstride < bw {
                    Err("invalid locked ANativeWindow_Buffer geometry".into())
                } else {
                    let dst_len = bstride * bh * 4;
                    let dst = std::slice::from_raw_parts_mut(buf.bits as *mut u8, dst_len);
                    cpu_blit_rgba(dst, bw, bh, bstride, frame, crop)
                };
                let rc_post = ANativeWindow_unlockAndPost(window);
                blit_res?;
                if rc_post != 0 {
                    return Err(format!("ANativeWindow_unlockAndPost failed ({rc_post})"));
                }
                Ok(RenderResult {
                    rendered: true,
                    mode,
                    backend: "cpu-software",
                    width: bw as u32,
                    height: bh as u32,
                    gl_renderer: "cpu-software".into(),
                    fallback_reason: Some(reason),
                })
            }
        }

        unsafe fn cpu_render_placeholder(
            window: *mut ANativeWindow,
            mode: &'static str,
            reason: String,
        ) -> Result<RenderResult, String> {
            if window.is_null() {
                return Err("null ANativeWindow".into());
            }
            unsafe {
                let mut w = ANativeWindow_getWidth(window);
                let mut h = ANativeWindow_getHeight(window);
                if w <= 0 || h <= 0 {
                    w = 64;
                    h = 64;
                }
                let rc_geom =
                    ANativeWindow_setBuffersGeometry(window, w, h, WINDOW_FORMAT_RGBA_8888);
                if rc_geom != 0 {
                    return Err(format!(
                        "ANativeWindow_setBuffersGeometry failed ({rc_geom})"
                    ));
                }
                let mut buf = ANativeWindowBuffer {
                    width: 0,
                    height: 0,
                    stride: 0,
                    format: 0,
                    bits: ptr::null_mut(),
                    reserved: [0; 6],
                };
                let rc_lock = ANativeWindow_lock(window, &mut buf, ptr::null_mut());
                if rc_lock != 0 || buf.bits.is_null() {
                    return Err(format!("ANativeWindow_lock failed ({rc_lock})"));
                }
                let bw = buf.width.max(0) as usize;
                let bh = buf.height.max(0) as usize;
                let bstride = buf.stride.max(0) as usize;
                if bw > 0 && bh > 0 && bstride >= bw {
                    let dst_len = bstride * bh * 4;
                    let dst = std::slice::from_raw_parts_mut(buf.bits as *mut u8, dst_len);
                    cpu_fill_placeholder(dst, bw, bh, bstride);
                }
                let rc_post = ANativeWindow_unlockAndPost(window);
                if rc_post != 0 {
                    return Err(format!("ANativeWindow_unlockAndPost failed ({rc_post})"));
                }
                Ok(RenderResult {
                    rendered: true,
                    mode,
                    backend: "cpu-software",
                    width: bw as u32,
                    height: bh as u32,
                    gl_renderer: "cpu-software".into(),
                    fallback_reason: Some(reason),
                })
            }
        }

        pub unsafe fn render_shm(
            &mut self,
            key: &str,
            window: *mut ANativeWindow,
            frame: ShmFrameRef<'_>,
            crop: [f32; 4],
        ) -> Result<RenderResult, String> {
            if let Some(reason) = self.gpu_disabled_reason.clone() {
                self.release_surface(key);
                return unsafe { Self::cpu_render_shm(window, frame, crop, "cpu-shm", reason) };
            }
            match unsafe { self.render_shm_egl(key, window, frame, crop) } {
                Ok(res) => Ok(res),
                Err(err) => {
                    self.release_surface(key);
                    self.gpu_disabled_reason = Some(err.clone());
                    unsafe { Self::cpu_render_shm(window, frame, crop, "cpu-shm", err) }
                }
            }
        }

        unsafe fn render_shm_egl(
            &mut self,
            key: &str,
            window: *mut ANativeWindow,
            frame: ShmFrameRef<'_>,
            crop: [f32; 4],
        ) -> Result<RenderResult, String> {
            if frame.width == 0 || frame.height == 0 || frame.stride < frame.width * 4 {
                return Err("invalid wl_shm frame dimensions".into());
            }
            let needed = frame.stride as usize * frame.height as usize;
            if frame.pixels.len() < needed {
                return Err("truncated wl_shm pixel buffer".into());
            }
            let (surf, surf_w, surf_h) = unsafe { self.ensure_window_surface(key, window) }?;
            unsafe {
                if eglMakeCurrent(self.display, surf, surf, self.context) == EGL_FALSE {
                    return Err(format!(
                        "eglMakeCurrent(window) failed (0x{:x})",
                        eglGetError()
                    ));
                }
                glViewport(0, 0, surf_w, surf_h);
                glClearColor(0.08, 0.09, 0.11, 1.0);
                glClear(GL_COLOR_BUFFER_BIT);
                glBindTexture(GL_TEXTURE_2D, self.shm_tex);
                glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
                let row_pixels = (frame.stride / 4) as i32;
                if self.is_gles3 {
                    glPixelStorei(GL_UNPACK_ROW_LENGTH, row_pixels);
                    glTexImage2D(
                        GL_TEXTURE_2D,
                        0,
                        GL_RGBA as i32,
                        frame.width as i32,
                        frame.height as i32,
                        0,
                        GL_RGBA,
                        GL_UNSIGNED_BYTE,
                        frame.pixels.as_ptr() as *const c_void,
                    );
                    glPixelStorei(GL_UNPACK_ROW_LENGTH, 0);
                } else if frame.stride == frame.width * 4 {
                    glTexImage2D(
                        GL_TEXTURE_2D,
                        0,
                        GL_RGBA as i32,
                        frame.width as i32,
                        frame.height as i32,
                        0,
                        GL_RGBA,
                        GL_UNSIGNED_BYTE,
                        frame.pixels.as_ptr() as *const c_void,
                    );
                } else {
                    let mut packed = vec![0u8; frame.width as usize * frame.height as usize * 4];
                    for r in 0..frame.height as usize {
                        let src_off = r * frame.stride as usize;
                        let dst_off = r * frame.width as usize * 4;
                        let row_len = frame.width as usize * 4;
                        packed[dst_off..dst_off + row_len]
                            .copy_from_slice(&frame.pixels[src_off..src_off + row_len]);
                    }
                    glTexImage2D(
                        GL_TEXTURE_2D,
                        0,
                        GL_RGBA as i32,
                        frame.width as i32,
                        frame.height as i32,
                        0,
                        GL_RGBA,
                        GL_UNSIGNED_BYTE,
                        packed.as_ptr() as *const c_void,
                    );
                }
                let swap_rb = matches!(
                    frame.wl_format,
                    WL_SHM_FORMAT_ARGB8888 | WL_SHM_FORMAT_XRGB8888
                );
                let opaque = frame.wl_format == WL_SHM_FORMAT_XRGB8888;
                if let Some(ref prog) = self.prog_2d {
                    prog.draw(GL_TEXTURE_2D, self.shm_tex, crop, swap_rb, opaque);
                }
                let swapped = eglSwapBuffers(self.display, surf);
                let swap_err = if swapped == EGL_FALSE {
                    eglGetError()
                } else {
                    0
                };
                eglMakeCurrent(self.display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
                if swapped == EGL_FALSE {
                    return Err(format!("eglSwapBuffers(shm) failed (0x{swap_err:x})"));
                }
            }
            Ok(RenderResult {
                rendered: true,
                mode: "shm",
                backend: self.active_gl_backend(),
                width: frame.width,
                height: frame.height,
                gl_renderer: self.egl_renderer.clone(),
                fallback_reason: None,
            })
        }

        pub unsafe fn render_dmabuf(
            &mut self,
            key: &str,
            window: *mut ANativeWindow,
            dmabuf: &DmabufDescriptor,
            crop: [f32; 4],
        ) -> Result<RenderResult, String> {
            let err_reason = match self.gpu_disabled_reason.clone() {
                Some(reason) => {
                    self.release_surface(key);
                    reason
                }
                None => match unsafe { self.render_dmabuf_egl(key, window, dmabuf, crop) } {
                    Ok(res) => return Ok(res),
                    Err((fatal_egl, err)) => {
                        self.release_surface(key);
                        if fatal_egl {
                            self.gpu_disabled_reason = Some(err.clone());
                        }
                        err
                    }
                },
            };

            // Fallback: if single-plane linear dmabuf is CPU-mappable, software-blit it;
            // otherwise render a CPU placeholder so the figure never stalls or crashes.
            if dmabuf.planes.len() == 1
                && dmabuf.width > 0
                && dmabuf.height > 0
                && dmabuf.planes[0].stride >= dmabuf.width * 4
                && matches!(dmabuf.planes[0].modifier, 0 | 0x00ff_ffff_ffff_ffff)
            {
                let p = &dmabuf.planes[0];
                if let Some(byte_len) = (p.offset as usize)
                    .checked_add((p.stride as usize).saturating_mul(dmabuf.height as usize))
                {
                    if byte_len > 0 && byte_len <= 64 * 1024 * 1024 {
                        let ptr = unsafe {
                            libc::mmap(
                                ptr::null_mut(),
                                byte_len,
                                libc::PROT_READ,
                                libc::MAP_SHARED,
                                p.fd.as_raw_fd(),
                                0,
                            )
                        };
                        if ptr != libc::MAP_FAILED {
                            let pixels =
                                unsafe { std::slice::from_raw_parts(ptr as *const u8, byte_len) };
                            let wl_format = match dmabuf.drm_format {
                                DRM_FORMAT_ARGB8888 => WL_SHM_FORMAT_ARGB8888,
                                DRM_FORMAT_XRGB8888 => WL_SHM_FORMAT_XRGB8888,
                                DRM_FORMAT_ABGR8888 | DRM_FORMAT_XBGR8888 => WL_SHM_FORMAT_ABGR8888,
                                _ => WL_SHM_FORMAT_ARGB8888,
                            };
                            let res = unsafe {
                                Self::cpu_render_shm(
                                    window,
                                    ShmFrameRef {
                                        width: dmabuf.width,
                                        height: dmabuf.height,
                                        stride: p.stride,
                                        wl_format,
                                        pixels: &pixels[p.offset as usize..],
                                    },
                                    crop,
                                    "cpu-dmabuf-mmap",
                                    err_reason.clone(),
                                )
                            };
                            unsafe { libc::munmap(ptr, byte_len) };
                            if let Ok(ok_res) = res {
                                return Ok(ok_res);
                            }
                        }
                    }
                }
            }
            unsafe { Self::cpu_render_placeholder(window, "cpu-dmabuf-fallback", err_reason) }
        }

        unsafe fn render_dmabuf_egl(
            &mut self,
            key: &str,
            window: *mut ANativeWindow,
            dmabuf: &DmabufDescriptor,
            crop: [f32; 4],
        ) -> Result<RenderResult, (bool, String)> {
            let (surf, surf_w, surf_h) =
                unsafe { self.ensure_window_surface(key, window) }.map_err(|e| (true, e))?;
            if !has_ext(&self.egl_exts, "EGL_EXT_image_dma_buf_import") {
                return Err((false, "EGL_EXT_image_dma_buf_import unavailable".into()));
            }
            let (Some(create_img), Some(destroy_img), Some(target_tex), Some(ref prog)) = (
                self.fn_create_image,
                self.fn_destroy_image,
                self.fn_image_target_tex,
                &self.prog_ext,
            ) else {
                return Err((
                    false,
                    "EGLImage OES external texture support unavailable".into(),
                ));
            };
            if dmabuf.planes.is_empty() || dmabuf.planes.len() > 3 {
                return Err((false, "unsupported dma-buf plane count".into()));
            }

            let with_mods = has_ext(&self.egl_exts, "EGL_EXT_image_dma_buf_import_modifiers");
            let mut attrs = vec![
                EGL_WIDTH,
                dmabuf.width as i32,
                EGL_HEIGHT,
                dmabuf.height as i32,
                EGL_LINUX_DRM_FOURCC_EXT,
                dmabuf.drm_format as i32,
            ];
            let plane_tags = [
                (
                    EGL_DMA_BUF_PLANE0_FD_EXT,
                    EGL_DMA_BUF_PLANE0_OFFSET_EXT,
                    EGL_DMA_BUF_PLANE0_PITCH_EXT,
                    EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT,
                    EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT,
                ),
                (
                    EGL_DMA_BUF_PLANE1_FD_EXT,
                    EGL_DMA_BUF_PLANE1_OFFSET_EXT,
                    EGL_DMA_BUF_PLANE1_PITCH_EXT,
                    EGL_DMA_BUF_PLANE1_MODIFIER_LO_EXT,
                    EGL_DMA_BUF_PLANE1_MODIFIER_HI_EXT,
                ),
                (
                    EGL_DMA_BUF_PLANE2_FD_EXT,
                    EGL_DMA_BUF_PLANE2_OFFSET_EXT,
                    EGL_DMA_BUF_PLANE2_PITCH_EXT,
                    EGL_DMA_BUF_PLANE2_MODIFIER_LO_EXT,
                    EGL_DMA_BUF_PLANE2_MODIFIER_HI_EXT,
                ),
            ];
            for p in &dmabuf.planes {
                let idx = p.plane_idx as usize;
                if idx >= plane_tags.len() {
                    return Err((false, "dma-buf plane index >= 3".into()));
                }
                let (t_fd, t_off, t_pitch, t_mlo, t_mhi) = plane_tags[idx];
                attrs.extend_from_slice(&[
                    t_fd,
                    p.fd.as_raw_fd(),
                    t_off,
                    p.offset as i32,
                    t_pitch,
                    p.stride as i32,
                ]);
                if with_mods && p.modifier != 0x00ff_ffff_ffff_ffff {
                    attrs.extend_from_slice(&[
                        t_mlo,
                        (p.modifier & 0xffff_ffff) as i32,
                        t_mhi,
                        (p.modifier >> 32) as i32,
                    ]);
                }
            }
            attrs.push(EGL_NONE);

            unsafe {
                if eglMakeCurrent(self.display, surf, surf, self.context) == EGL_FALSE {
                    return Err((
                        true,
                        format!("eglMakeCurrent(window) failed (0x{:x})", eglGetError()),
                    ));
                }
                let img = create_img(
                    self.display,
                    EGL_NO_CONTEXT,
                    EGL_LINUX_DMA_BUF_EXT,
                    ptr::null_mut(),
                    attrs.as_ptr(),
                );
                if img == EGL_NO_IMAGE_KHR {
                    let err = eglGetError();
                    eglMakeCurrent(self.display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
                    return Err((
                        false,
                        format!("eglCreateImageKHR(dmabuf) failed (0x{err:x})"),
                    ));
                }
                glViewport(0, 0, surf_w, surf_h);
                glClearColor(0.08, 0.09, 0.11, 1.0);
                glClear(GL_COLOR_BUFFER_BIT);
                glBindTexture(GL_TEXTURE_EXTERNAL_OES, self.ext_tex);
                target_tex(GL_TEXTURE_EXTERNAL_OES, img);
                let swap_rb =
                    matches!(dmabuf.drm_format, DRM_FORMAT_ARGB8888 | DRM_FORMAT_XRGB8888);
                let opaque = matches!(dmabuf.drm_format, DRM_FORMAT_XRGB8888 | DRM_FORMAT_XBGR8888);
                prog.draw(GL_TEXTURE_EXTERNAL_OES, self.ext_tex, crop, swap_rb, opaque);
                let swapped = eglSwapBuffers(self.display, surf);
                let swap_err = if swapped == EGL_FALSE {
                    eglGetError()
                } else {
                    0
                };
                destroy_img(self.display, img);
                eglMakeCurrent(self.display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
                if swapped == EGL_FALSE {
                    return Err((
                        true,
                        format!("eglSwapBuffers(dmabuf) failed (0x{swap_err:x})"),
                    ));
                }
            }
            Ok(RenderResult {
                rendered: true,
                mode: "dmabuf",
                backend: self.active_gl_backend(),
                width: dmabuf.width,
                height: dmabuf.height,
                gl_renderer: self.egl_renderer.clone(),
                fallback_reason: None,
            })
        }

        pub unsafe fn render_placeholder(
            &mut self,
            key: &str,
            window: *mut ANativeWindow,
        ) -> Result<RenderResult, String> {
            if let Some(reason) = self.gpu_disabled_reason.clone() {
                self.release_surface(key);
                return unsafe { Self::cpu_render_placeholder(window, "cpu-placeholder", reason) };
            }
            match unsafe { self.render_placeholder_egl(key, window) } {
                Ok(res) => Ok(res),
                Err(err) => {
                    self.release_surface(key);
                    self.gpu_disabled_reason = Some(err.clone());
                    unsafe { Self::cpu_render_placeholder(window, "cpu-placeholder", err) }
                }
            }
        }

        unsafe fn render_placeholder_egl(
            &mut self,
            key: &str,
            window: *mut ANativeWindow,
        ) -> Result<RenderResult, String> {
            let (surf, surf_w, surf_h) = unsafe { self.ensure_window_surface(key, window) }?;
            unsafe {
                if eglMakeCurrent(self.display, surf, surf, self.context) == EGL_FALSE {
                    return Err(format!(
                        "eglMakeCurrent(window) failed (0x{:x})",
                        eglGetError()
                    ));
                }
                glViewport(0, 0, surf_w, surf_h);
                glClearColor(0.08, 0.10, 0.12, 1.0);
                glClear(GL_COLOR_BUFFER_BIT);
                let swapped = eglSwapBuffers(self.display, surf);
                let swap_err = if swapped == EGL_FALSE {
                    eglGetError()
                } else {
                    0
                };
                eglMakeCurrent(self.display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
                if swapped == EGL_FALSE {
                    return Err(format!(
                        "eglSwapBuffers(placeholder) failed (0x{swap_err:x})"
                    ));
                }
            }
            Ok(RenderResult {
                rendered: true,
                mode: "placeholder",
                backend: self.active_gl_backend(),
                width: surf_w as u32,
                height: surf_h as u32,
                gl_renderer: self.egl_renderer.clone(),
                fallback_reason: None,
            })
        }
    }

    impl Drop for AndroidGpuRenderer {
        fn drop(&mut self) {
            let keys: Vec<String> = self.surfaces.keys().cloned().collect();
            for k in keys {
                self.release_surface(&k);
            }
            unsafe {
                if self.display != EGL_NO_DISPLAY {
                    if self.context != EGL_NO_CONTEXT {
                        if self.pbuffer != EGL_NO_SURFACE
                            && eglMakeCurrent(
                                self.display,
                                self.pbuffer,
                                self.pbuffer,
                                self.context,
                            ) == EGL_TRUE
                        {
                            let texs = [self.shm_tex, self.ext_tex];
                            glDeleteTextures(2, texs.as_ptr());
                            if let Some(p) = self.prog_2d.take() {
                                glDeleteProgram(p.program);
                            }
                            if let Some(p) = self.prog_ext.take() {
                                glDeleteProgram(p.program);
                            }
                            eglMakeCurrent(
                                self.display,
                                EGL_NO_SURFACE,
                                EGL_NO_SURFACE,
                                EGL_NO_CONTEXT,
                            );
                        }
                        eglDestroyContext(self.display, self.context);
                    }
                    if self.pbuffer != EGL_NO_SURFACE {
                        eglDestroySurface(self.display, self.pbuffer);
                    }
                    eglTerminate(self.display);
                }
            }
        }
    }

    fn has_ext(list: &str, needle: &str) -> bool {
        list.split_whitespace().any(|ext| ext == needle)
    }

    // ---- Vulkan 1.1 dynamic loader + compute + AHardwareBuffer interop ----

    type VkResult = i32;
    type VkFlags = u32;
    type VkBool32 = u32;
    type VkDeviceSize = u64;
    type VkInstance = *mut c_void;
    type VkPhysicalDevice = *mut c_void;
    type VkDevice = *mut c_void;
    type VkQueue = *mut c_void;
    type VkBuffer = u64;
    type VkDeviceMemory = u64;
    type VkShaderModule = u64;
    type VkDescriptorSetLayout = u64;
    type VkPipelineLayout = u64;
    type VkPipelineCache = u64;
    type VkPipeline = u64;
    type VkDescriptorPool = u64;
    type VkDescriptorSet = u64;
    type VkCommandPool = u64;
    type VkCommandBuffer = *mut c_void;
    type VkFence = u64;

    const VK_SUCCESS: VkResult = 0;
    const VK_STRUCTURE_TYPE_APPLICATION_INFO: u32 = 0;
    const VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO: u32 = 1;
    const VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO: u32 = 2;
    const VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO: u32 = 3;
    const VK_STRUCTURE_TYPE_SUBMIT_INFO: u32 = 4;
    const VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO: u32 = 5;
    const VK_STRUCTURE_TYPE_FENCE_CREATE_INFO: u32 = 8;
    const VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO: u32 = 12;
    const VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO: u32 = 16;
    const VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO: u32 = 18;
    const VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO: u32 = 29;
    const VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO: u32 = 30;
    const VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO: u32 = 32;
    const VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO: u32 = 33;
    const VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO: u32 = 34;
    const VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET: u32 = 35;
    const VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO: u32 = 39;
    const VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO: u32 = 40;
    const VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO: u32 = 42;
    const VK_STRUCTURE_TYPE_ANDROID_HARDWARE_BUFFER_PROPERTIES_ANDROID: u32 = 1000129001;
    const VK_STRUCTURE_TYPE_ANDROID_HARDWARE_BUFFER_FORMAT_PROPERTIES_ANDROID: u32 = 1000129002;

    const VK_QUEUE_COMPUTE_BIT: VkFlags = 0x0000_0002;
    const VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT: VkFlags = 0x0000_0002;
    const VK_MEMORY_PROPERTY_HOST_COHERENT_BIT: VkFlags = 0x0000_0004;
    const VK_BUFFER_USAGE_STORAGE_BUFFER_BIT: VkFlags = 0x0000_0020;
    const VK_SHADER_STAGE_COMPUTE_BIT: VkFlags = 0x0000_0020;
    const VK_DESCRIPTOR_TYPE_STORAGE_BUFFER: u32 = 7;
    const VK_PIPELINE_BIND_POINT_COMPUTE: u32 = 1;
    const VK_COMMAND_BUFFER_LEVEL_PRIMARY: u32 = 0;
    const VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT: VkFlags = 0x0000_0001;
    const VK_SHARING_MODE_EXCLUSIVE: u32 = 0;
    const VK_API_VERSION_1_1: u32 = (1 << 22) | (1 << 12);

    #[repr(C)]
    struct VkApplicationInfo {
        s_type: u32,
        p_next: *const c_void,
        p_application_name: *const c_char,
        application_version: u32,
        p_engine_name: *const c_char,
        engine_version: u32,
        api_version: u32,
    }

    #[repr(C)]
    struct VkInstanceCreateInfo {
        s_type: u32,
        p_next: *const c_void,
        flags: VkFlags,
        p_application_info: *const VkApplicationInfo,
        enabled_layer_count: u32,
        pp_enabled_layer_names: *const *const c_char,
        enabled_extension_count: u32,
        pp_enabled_extension_names: *const *const c_char,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct VkExtent3D {
        width: u32,
        height: u32,
        depth: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct VkQueueFamilyProperties {
        queue_flags: VkFlags,
        queue_count: u32,
        timestamp_valid_bits: u32,
        min_image_transfer_granularity: VkExtent3D,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct VkExtensionProperties {
        extension_name: [u8; 256],
        spec_version: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct VkPhysicalDeviceLimits {
        max_image_dimension_1d: u32,
        max_image_dimension_2d: u32,
        max_image_dimension_3d: u32,
        max_image_dimension_cube: u32,
        max_image_array_layers: u32,
        max_texel_buffer_elements: u32,
        max_uniform_buffer_range: u32,
        max_storage_buffer_range: u32,
        max_push_constants_size: u32,
        max_memory_allocation_count: u32,
        max_sampler_allocation_count: u32,
        buffer_image_granularity: VkDeviceSize,
        sparse_address_space_size: VkDeviceSize,
        max_bound_descriptor_sets: u32,
        max_per_stage_descriptor_samplers: u32,
        max_per_stage_descriptor_uniform_buffers: u32,
        max_per_stage_descriptor_storage_buffers: u32,
        max_per_stage_descriptor_sampled_images: u32,
        max_per_stage_descriptor_storage_images: u32,
        max_per_stage_descriptor_input_attachments: u32,
        max_per_stage_resources: u32,
        max_descriptor_set_samplers: u32,
        max_descriptor_set_uniform_buffers: u32,
        max_descriptor_set_uniform_buffers_dynamic: u32,
        max_descriptor_set_storage_buffers: u32,
        max_descriptor_set_storage_buffers_dynamic: u32,
        max_descriptor_set_sampled_images: u32,
        max_descriptor_set_storage_images: u32,
        max_descriptor_set_input_attachments: u32,
        max_vertex_input_attributes: u32,
        max_vertex_input_bindings: u32,
        max_vertex_input_attribute_offset: u32,
        max_vertex_input_binding_stride: u32,
        max_vertex_output_components: u32,
        max_tessellation_generation_level: u32,
        max_tessellation_patch_size: u32,
        max_tessellation_control_per_vertex_input_components: u32,
        max_tessellation_control_per_vertex_output_components: u32,
        max_tessellation_control_per_patch_output_components: u32,
        max_tessellation_control_total_output_components: u32,
        max_tessellation_evaluation_input_components: u32,
        max_tessellation_evaluation_output_components: u32,
        max_geometry_shader_invocations: u32,
        max_geometry_input_components: u32,
        max_geometry_output_components: u32,
        max_geometry_output_vertices: u32,
        max_geometry_total_output_components: u32,
        max_fragment_input_components: u32,
        max_fragment_output_attachments: u32,
        max_fragment_dual_src_attachments: u32,
        max_fragment_combined_output_resources: u32,
        max_compute_shared_memory_size: u32,
        max_compute_work_group_count: [u32; 3],
        max_compute_work_group_invocations: u32,
        max_compute_work_group_size: [u32; 3],
        sub_pixel_precision_bits: u32,
        sub_texel_precision_bits: u32,
        mipmap_precision_bits: u32,
        max_draw_indexed_index_value: u32,
        max_draw_indirect_count: u32,
        max_sampler_lod_bias: f32,
        max_sampler_anisotropy: f32,
        max_viewports: u32,
        max_viewport_dimensions: [u32; 2],
        viewport_bounds_range: [f32; 2],
        viewport_sub_pixel_bits: u32,
        min_memory_map_alignment: usize,
        min_texel_buffer_offset_alignment: VkDeviceSize,
        min_uniform_buffer_offset_alignment: VkDeviceSize,
        min_storage_buffer_offset_alignment: VkDeviceSize,
        min_texel_offset: i32,
        max_texel_offset: u32,
        min_texel_gather_offset: i32,
        max_texel_gather_offset: u32,
        min_interpolation_offset: f32,
        max_interpolation_offset: f32,
        sub_pixel_interpolation_offset_bits: u32,
        max_framebuffer_width: u32,
        max_framebuffer_height: u32,
        max_framebuffer_layers: u32,
        framebuffer_color_sample_counts: VkFlags,
        framebuffer_depth_sample_counts: VkFlags,
        framebuffer_stencil_sample_counts: VkFlags,
        framebuffer_no_attachments_sample_counts: VkFlags,
        max_color_attachments: u32,
        sampled_image_color_sample_counts: VkFlags,
        sampled_image_integer_sample_counts: VkFlags,
        sampled_image_depth_sample_counts: VkFlags,
        sampled_image_stencil_sample_counts: VkFlags,
        storage_image_sample_counts: VkFlags,
        max_sample_mask_words: u32,
        timestamp_compute_and_graphics: VkBool32,
        timestamp_period: f32,
        max_clip_distances: u32,
        max_cull_distances: u32,
        max_combined_clip_and_cull_distances: u32,
        discrete_queue_priorities: u32,
        point_size_range: [f32; 2],
        line_width_range: [f32; 2],
        point_size_granularity: f32,
        line_width_granularity: f32,
        strict_lines: VkBool32,
        standard_sample_locations: VkBool32,
        optimal_buffer_copy_offset_alignment: VkDeviceSize,
        optimal_buffer_copy_row_pitch_alignment: VkDeviceSize,
        non_coherent_atom_size: VkDeviceSize,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct VkPhysicalDeviceSparseProperties {
        a: VkBool32,
        b: VkBool32,
        c: VkBool32,
        d: VkBool32,
        e: VkBool32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct VkPhysicalDeviceProperties {
        api_version: u32,
        driver_version: u32,
        vendor_id: u32,
        device_id: u32,
        device_type: u32,
        device_name: [u8; 256],
        pipeline_cache_uuid: [u8; 16],
        limits: VkPhysicalDeviceLimits,
        sparse_properties: VkPhysicalDeviceSparseProperties,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct VkMemoryType {
        property_flags: VkFlags,
        heap_index: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct VkMemoryHeap {
        size: VkDeviceSize,
        flags: VkFlags,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct VkPhysicalDeviceMemoryProperties {
        memory_type_count: u32,
        memory_types: [VkMemoryType; 32],
        memory_heap_count: u32,
        memory_heaps: [VkMemoryHeap; 16],
    }

    #[repr(C)]
    struct VkDeviceQueueCreateInfo {
        s_type: u32,
        p_next: *const c_void,
        flags: VkFlags,
        queue_family_index: u32,
        queue_count: u32,
        p_queue_priorities: *const f32,
    }

    #[repr(C)]
    struct VkDeviceCreateInfo {
        s_type: u32,
        p_next: *const c_void,
        flags: VkFlags,
        queue_create_info_count: u32,
        p_queue_create_infos: *const VkDeviceQueueCreateInfo,
        enabled_layer_count: u32,
        pp_enabled_layer_names: *const *const c_char,
        enabled_extension_count: u32,
        pp_enabled_extension_names: *const *const c_char,
        p_enabled_features: *const c_void,
    }

    #[repr(C)]
    struct VkBufferCreateInfo {
        s_type: u32,
        p_next: *const c_void,
        flags: VkFlags,
        size: VkDeviceSize,
        usage: VkFlags,
        sharing_mode: u32,
        queue_family_index_count: u32,
        p_queue_family_indices: *const u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct VkMemoryRequirements {
        size: VkDeviceSize,
        alignment: VkDeviceSize,
        memory_type_bits: u32,
    }

    #[repr(C)]
    struct VkMemoryAllocateInfo {
        s_type: u32,
        p_next: *const c_void,
        allocation_size: VkDeviceSize,
        memory_type_index: u32,
    }

    #[repr(C)]
    struct VkShaderModuleCreateInfo {
        s_type: u32,
        p_next: *const c_void,
        flags: VkFlags,
        code_size: usize,
        p_code: *const u32,
    }

    #[repr(C)]
    struct VkDescriptorSetLayoutBinding {
        binding: u32,
        descriptor_type: u32,
        descriptor_count: u32,
        stage_flags: VkFlags,
        p_immutable_samplers: *const c_void,
    }

    #[repr(C)]
    struct VkDescriptorSetLayoutCreateInfo {
        s_type: u32,
        p_next: *const c_void,
        flags: VkFlags,
        binding_count: u32,
        p_bindings: *const VkDescriptorSetLayoutBinding,
    }

    #[repr(C)]
    struct VkPipelineLayoutCreateInfo {
        s_type: u32,
        p_next: *const c_void,
        flags: VkFlags,
        set_layout_count: u32,
        p_set_layouts: *const VkDescriptorSetLayout,
        push_constant_range_count: u32,
        p_push_constant_ranges: *const c_void,
    }

    #[repr(C)]
    struct VkPipelineShaderStageCreateInfo {
        s_type: u32,
        p_next: *const c_void,
        flags: VkFlags,
        stage: VkFlags,
        module: VkShaderModule,
        p_name: *const c_char,
        p_specialization_info: *const c_void,
    }

    #[repr(C)]
    struct VkComputePipelineCreateInfo {
        s_type: u32,
        p_next: *const c_void,
        flags: VkFlags,
        stage: VkPipelineShaderStageCreateInfo,
        layout: VkPipelineLayout,
        base_pipeline_handle: VkPipeline,
        base_pipeline_index: i32,
    }

    #[repr(C)]
    struct VkDescriptorPoolSize {
        descriptor_type: u32,
        descriptor_count: u32,
    }

    #[repr(C)]
    struct VkDescriptorPoolCreateInfo {
        s_type: u32,
        p_next: *const c_void,
        flags: VkFlags,
        max_sets: u32,
        pool_size_count: u32,
        p_pool_sizes: *const VkDescriptorPoolSize,
    }

    #[repr(C)]
    struct VkDescriptorSetAllocateInfo {
        s_type: u32,
        p_next: *const c_void,
        descriptor_pool: VkDescriptorPool,
        descriptor_set_count: u32,
        p_set_layouts: *const VkDescriptorSetLayout,
    }

    #[repr(C)]
    struct VkDescriptorBufferInfo {
        buffer: VkBuffer,
        offset: VkDeviceSize,
        range: VkDeviceSize,
    }

    #[repr(C)]
    struct VkWriteDescriptorSet {
        s_type: u32,
        p_next: *const c_void,
        dst_set: VkDescriptorSet,
        dst_binding: u32,
        dst_array_element: u32,
        descriptor_count: u32,
        descriptor_type: u32,
        p_image_info: *const c_void,
        p_buffer_info: *const VkDescriptorBufferInfo,
        p_texel_buffer_view: *const c_void,
    }

    #[repr(C)]
    struct VkCommandPoolCreateInfo {
        s_type: u32,
        p_next: *const c_void,
        flags: VkFlags,
        queue_family_index: u32,
    }

    #[repr(C)]
    struct VkCommandBufferAllocateInfo {
        s_type: u32,
        p_next: *const c_void,
        command_pool: VkCommandPool,
        level: u32,
        command_buffer_count: u32,
    }

    #[repr(C)]
    struct VkCommandBufferBeginInfo {
        s_type: u32,
        p_next: *const c_void,
        flags: VkFlags,
        p_inheritance_info: *const c_void,
    }

    #[repr(C)]
    struct VkSubmitInfo {
        s_type: u32,
        p_next: *const c_void,
        wait_semaphore_count: u32,
        p_wait_semaphores: *const c_void,
        p_wait_dst_stage_mask: *const VkFlags,
        command_buffer_count: u32,
        p_command_buffers: *const VkCommandBuffer,
        signal_semaphore_count: u32,
        p_signal_semaphores: *const c_void,
    }

    #[repr(C)]
    struct VkFenceCreateInfo {
        s_type: u32,
        p_next: *const c_void,
        flags: VkFlags,
    }

    #[repr(C)]
    struct VkAndroidHardwareBufferFormatPropertiesANDROID {
        s_type: u32,
        p_next: *mut c_void,
        format: u32,
        external_format: u64,
        format_features: u32,
        sampler_ycbcr_conversion_components: [u32; 4],
        suggested_ycbcr_model: u32,
        suggested_ycbcr_range: u32,
        suggested_x_chroma_offset: u32,
        suggested_y_chroma_offset: u32,
    }

    #[repr(C)]
    struct VkAndroidHardwareBufferPropertiesANDROID {
        s_type: u32,
        p_next: *mut c_void,
        allocation_size: VkDeviceSize,
        memory_type_bits: u32,
    }

    type FnVkGetInstanceProcAddr =
        unsafe extern "C" fn(instance: VkInstance, name: *const c_char) -> *const c_void;
    type FnVkGetDeviceProcAddr =
        unsafe extern "C" fn(device: VkDevice, name: *const c_char) -> *const c_void;
    type FnVkCreateInstance = unsafe extern "C" fn(
        *const VkInstanceCreateInfo,
        *const c_void,
        *mut VkInstance,
    ) -> VkResult;
    type FnVkDestroyInstance = unsafe extern "C" fn(VkInstance, *const c_void);
    type FnVkEnumeratePhysicalDevices =
        unsafe extern "C" fn(VkInstance, *mut u32, *mut VkPhysicalDevice) -> VkResult;
    type FnVkGetPhysicalDeviceProperties =
        unsafe extern "C" fn(VkPhysicalDevice, *mut VkPhysicalDeviceProperties);
    type FnVkGetPhysicalDeviceMemoryProperties =
        unsafe extern "C" fn(VkPhysicalDevice, *mut VkPhysicalDeviceMemoryProperties);
    type FnVkGetPhysicalDeviceQueueFamilyProperties =
        unsafe extern "C" fn(VkPhysicalDevice, *mut u32, *mut VkQueueFamilyProperties);
    type FnVkEnumerateDeviceExtensionProperties = unsafe extern "C" fn(
        VkPhysicalDevice,
        *const c_char,
        *mut u32,
        *mut VkExtensionProperties,
    ) -> VkResult;
    type FnVkCreateDevice = unsafe extern "C" fn(
        VkPhysicalDevice,
        *const VkDeviceCreateInfo,
        *const c_void,
        *mut VkDevice,
    ) -> VkResult;
    type FnVkDestroyDevice = unsafe extern "C" fn(VkDevice, *const c_void);
    type FnVkGetDeviceQueue = unsafe extern "C" fn(VkDevice, u32, u32, *mut VkQueue);
    type FnVkCreateBuffer = unsafe extern "C" fn(
        VkDevice,
        *const VkBufferCreateInfo,
        *const c_void,
        *mut VkBuffer,
    ) -> VkResult;
    type FnVkDestroyBuffer = unsafe extern "C" fn(VkDevice, VkBuffer, *const c_void);
    type FnVkGetBufferMemoryRequirements =
        unsafe extern "C" fn(VkDevice, VkBuffer, *mut VkMemoryRequirements);
    type FnVkAllocateMemory = unsafe extern "C" fn(
        VkDevice,
        *const VkMemoryAllocateInfo,
        *const c_void,
        *mut VkDeviceMemory,
    ) -> VkResult;
    type FnVkFreeMemory = unsafe extern "C" fn(VkDevice, VkDeviceMemory, *const c_void);
    type FnVkBindBufferMemory =
        unsafe extern "C" fn(VkDevice, VkBuffer, VkDeviceMemory, VkDeviceSize) -> VkResult;
    type FnVkMapMemory = unsafe extern "C" fn(
        VkDevice,
        VkDeviceMemory,
        VkDeviceSize,
        VkDeviceSize,
        VkFlags,
        *mut *mut c_void,
    ) -> VkResult;
    type FnVkUnmapMemory = unsafe extern "C" fn(VkDevice, VkDeviceMemory);
    type FnVkCreateShaderModule = unsafe extern "C" fn(
        VkDevice,
        *const VkShaderModuleCreateInfo,
        *const c_void,
        *mut VkShaderModule,
    ) -> VkResult;
    type FnVkDestroyShaderModule = unsafe extern "C" fn(VkDevice, VkShaderModule, *const c_void);
    type FnVkCreateDescriptorSetLayout = unsafe extern "C" fn(
        VkDevice,
        *const VkDescriptorSetLayoutCreateInfo,
        *const c_void,
        *mut VkDescriptorSetLayout,
    ) -> VkResult;
    type FnVkDestroyDescriptorSetLayout =
        unsafe extern "C" fn(VkDevice, VkDescriptorSetLayout, *const c_void);
    type FnVkCreatePipelineLayout = unsafe extern "C" fn(
        VkDevice,
        *const VkPipelineLayoutCreateInfo,
        *const c_void,
        *mut VkPipelineLayout,
    ) -> VkResult;
    type FnVkDestroyPipelineLayout =
        unsafe extern "C" fn(VkDevice, VkPipelineLayout, *const c_void);
    type FnVkCreateComputePipelines = unsafe extern "C" fn(
        VkDevice,
        VkPipelineCache,
        u32,
        *const VkComputePipelineCreateInfo,
        *const c_void,
        *mut VkPipeline,
    ) -> VkResult;
    type FnVkDestroyPipeline = unsafe extern "C" fn(VkDevice, VkPipeline, *const c_void);
    type FnVkCreateDescriptorPool = unsafe extern "C" fn(
        VkDevice,
        *const VkDescriptorPoolCreateInfo,
        *const c_void,
        *mut VkDescriptorPool,
    ) -> VkResult;
    type FnVkDestroyDescriptorPool =
        unsafe extern "C" fn(VkDevice, VkDescriptorPool, *const c_void);
    type FnVkAllocateDescriptorSets = unsafe extern "C" fn(
        VkDevice,
        *const VkDescriptorSetAllocateInfo,
        *mut VkDescriptorSet,
    ) -> VkResult;
    type FnVkUpdateDescriptorSets =
        unsafe extern "C" fn(VkDevice, u32, *const VkWriteDescriptorSet, u32, *const c_void);
    type FnVkCreateCommandPool = unsafe extern "C" fn(
        VkDevice,
        *const VkCommandPoolCreateInfo,
        *const c_void,
        *mut VkCommandPool,
    ) -> VkResult;
    type FnVkDestroyCommandPool = unsafe extern "C" fn(VkDevice, VkCommandPool, *const c_void);
    type FnVkAllocateCommandBuffers = unsafe extern "C" fn(
        VkDevice,
        *const VkCommandBufferAllocateInfo,
        *mut VkCommandBuffer,
    ) -> VkResult;
    type FnVkBeginCommandBuffer =
        unsafe extern "C" fn(VkCommandBuffer, *const VkCommandBufferBeginInfo) -> VkResult;
    type FnVkEndCommandBuffer = unsafe extern "C" fn(VkCommandBuffer) -> VkResult;
    type FnVkCmdBindPipeline = unsafe extern "C" fn(VkCommandBuffer, u32, VkPipeline);
    type FnVkCmdBindDescriptorSets = unsafe extern "C" fn(
        VkCommandBuffer,
        u32,
        VkPipelineLayout,
        u32,
        u32,
        *const VkDescriptorSet,
        u32,
        *const u32,
    );
    type FnVkCmdDispatch = unsafe extern "C" fn(VkCommandBuffer, u32, u32, u32);
    type FnVkQueueSubmit =
        unsafe extern "C" fn(VkQueue, u32, *const VkSubmitInfo, VkFence) -> VkResult;
    type FnVkCreateFence = unsafe extern "C" fn(
        VkDevice,
        *const VkFenceCreateInfo,
        *const c_void,
        *mut VkFence,
    ) -> VkResult;
    type FnVkDestroyFence = unsafe extern "C" fn(VkDevice, VkFence, *const c_void);
    type FnVkWaitForFences =
        unsafe extern "C" fn(VkDevice, u32, *const VkFence, VkBool32, u64) -> VkResult;
    type FnVkGetAndroidHardwareBufferPropertiesANDROID = unsafe extern "C" fn(
        VkDevice,
        *const AHardwareBuffer,
        *mut VkAndroidHardwareBufferPropertiesANDROID,
    ) -> VkResult;

    struct VkLoader {
        lib: *mut c_void,
        get_inst_proc: FnVkGetInstanceProcAddr,
        instance: VkInstance,
    }

    impl VkLoader {
        unsafe fn new() -> Result<Self, String> {
            unsafe {
                let lib = libc::dlopen(c"libvulkan.so".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
                if lib.is_null() {
                    return Err("libvulkan.so not available".into());
                }
                let sym = libc::dlsym(lib, c"vkGetInstanceProcAddr".as_ptr());
                if sym.is_null() {
                    libc::dlclose(lib);
                    return Err("vkGetInstanceProcAddr missing".into());
                }
                let get_inst_proc: FnVkGetInstanceProcAddr = std::mem::transmute(sym);
                let p_create = get_inst_proc(ptr::null_mut(), c"vkCreateInstance".as_ptr());
                if p_create.is_null() {
                    libc::dlclose(lib);
                    return Err("vkCreateInstance missing".into());
                }
                let create_inst: FnVkCreateInstance = std::mem::transmute(p_create);
                let app = VkApplicationInfo {
                    s_type: VK_STRUCTURE_TYPE_APPLICATION_INFO,
                    p_next: ptr::null(),
                    p_application_name: c"emthin-pad".as_ptr(),
                    application_version: 1,
                    p_engine_name: c"emthin".as_ptr(),
                    engine_version: 1,
                    api_version: VK_API_VERSION_1_1,
                };
                let ici = VkInstanceCreateInfo {
                    s_type: VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO,
                    p_next: ptr::null(),
                    flags: 0,
                    p_application_info: &app,
                    enabled_layer_count: 0,
                    pp_enabled_layer_names: ptr::null(),
                    enabled_extension_count: 0,
                    pp_enabled_extension_names: ptr::null(),
                };
                let mut instance: VkInstance = ptr::null_mut();
                let rc = create_inst(&ici, ptr::null(), &mut instance);
                if rc != VK_SUCCESS || instance.is_null() {
                    libc::dlclose(lib);
                    return Err(format!("vkCreateInstance failed ({rc})"));
                }
                Ok(Self {
                    lib,
                    get_inst_proc,
                    instance,
                })
            }
        }

        unsafe fn inst_fn<T>(&self, name: &CStr) -> Result<T, String> {
            unsafe {
                let p = (self.get_inst_proc)(self.instance, name.as_ptr());
                if p.is_null() {
                    return Err(format!("missing Vulkan symbol {}", name.to_string_lossy()));
                }
                Ok(std::mem::transmute_copy(&p))
            }
        }
    }

    impl Drop for VkLoader {
        fn drop(&mut self) {
            unsafe {
                if !self.instance.is_null() {
                    let p = (self.get_inst_proc)(self.instance, c"vkDestroyInstance".as_ptr());
                    if !p.is_null() {
                        let destroy: FnVkDestroyInstance = std::mem::transmute(p);
                        destroy(self.instance, ptr::null());
                    }
                }
                if !self.lib.is_null() {
                    libc::dlclose(self.lib);
                }
            }
        }
    }

    struct SelectedDevice {
        phys: VkPhysicalDevice,
        name: String,
        device_type: String,
        api_version: String,
        compute_family: u32,
        max_invocations: u32,
        has_ahb: bool,
        has_dmabuf: bool,
    }

    unsafe fn select_compute_device(vk: &VkLoader) -> Result<SelectedDevice, String> {
        unsafe {
            let enum_devs: FnVkEnumeratePhysicalDevices =
                vk.inst_fn(c"vkEnumeratePhysicalDevices")?;
            let get_props: FnVkGetPhysicalDeviceProperties =
                vk.inst_fn(c"vkGetPhysicalDeviceProperties")?;
            let get_qprops: FnVkGetPhysicalDeviceQueueFamilyProperties =
                vk.inst_fn(c"vkGetPhysicalDeviceQueueFamilyProperties")?;
            let enum_exts: FnVkEnumerateDeviceExtensionProperties =
                vk.inst_fn(c"vkEnumerateDeviceExtensionProperties")?;

            let mut count = 0u32;
            let rc = enum_devs(vk.instance, &mut count, ptr::null_mut());
            if rc != VK_SUCCESS || count == 0 {
                return Err(format!("no Vulkan physical devices (rc={rc})"));
            }
            let mut devs = vec![ptr::null_mut(); count as usize];
            let rc = enum_devs(vk.instance, &mut count, devs.as_mut_ptr());
            if rc != VK_SUCCESS {
                return Err(format!("vkEnumeratePhysicalDevices failed ({rc})"));
            }

            let mut best: Option<(i32, SelectedDevice)> = None;
            for phys in devs {
                let mut props: VkPhysicalDeviceProperties = std::mem::zeroed();
                get_props(phys, &mut props);
                let mut qcount = 0u32;
                get_qprops(phys, &mut qcount, ptr::null_mut());
                let mut qprops =
                    vec![std::mem::zeroed::<VkQueueFamilyProperties>(); qcount as usize];
                get_qprops(phys, &mut qcount, qprops.as_mut_ptr());
                let Some(qf) = qprops
                    .iter()
                    .position(|q| (q.queue_flags & VK_QUEUE_COMPUTE_BIT) != 0 && q.queue_count > 0)
                else {
                    continue;
                };

                let mut ecount = 0u32;
                let _ = enum_exts(phys, ptr::null(), &mut ecount, ptr::null_mut());
                let mut exts = vec![std::mem::zeroed::<VkExtensionProperties>(); ecount as usize];
                if ecount > 0 {
                    let _ = enum_exts(phys, ptr::null(), &mut ecount, exts.as_mut_ptr());
                }
                let has_ext_name = |want: &str| {
                    exts.iter().any(|e| {
                        let end = e.extension_name.iter().position(|&b| b == 0).unwrap_or(256);
                        std::str::from_utf8(&e.extension_name[..end]).ok() == Some(want)
                    })
                };
                let has_ahb = has_ext_name("VK_ANDROID_external_memory_android_hardware_buffer");
                let has_dmabuf = has_ext_name("VK_EXT_external_memory_dma_buf");

                let (score, kind) = match props.device_type {
                    2 => (100, "discrete"),
                    1 => (90, "integrated"),
                    3 => (60, "virtual"),
                    4 => (10, "cpu"),
                    _ => (20, "other"),
                };
                let name_end = props
                    .device_name
                    .iter()
                    .position(|&b| b == 0)
                    .unwrap_or(256);
                let name = String::from_utf8_lossy(&props.device_name[..name_end]).into_owned();
                let ver = props.api_version;
                let api_version = format!(
                    "{}.{}.{}",
                    (ver >> 22) & 0x7f,
                    (ver >> 12) & 0x3ff,
                    ver & 0xfff
                );
                let cand = SelectedDevice {
                    phys,
                    name,
                    device_type: kind.into(),
                    api_version,
                    compute_family: qf as u32,
                    max_invocations: props.limits.max_compute_work_group_invocations,
                    has_ahb,
                    has_dmabuf,
                };
                if best.as_ref().is_none_or(|(s, _)| score > *s) {
                    best = Some((score, cand));
                }
            }
            best.map(|(_, d)| d)
                .ok_or_else(|| "no Vulkan compute queue family found".into())
        }
    }

    pub fn fill_vulkan_capabilities(caps: &mut GpuCapabilities) {
        if let Ok(vk) = unsafe { VkLoader::new() } {
            if let Ok(dev) = unsafe { select_compute_device(&vk) } {
                caps.has_vulkan = true;
                caps.vulkan_device_name = dev.name;
                caps.vulkan_device_type = dev.device_type;
                caps.vulkan_api_version = dev.api_version;
                caps.vulkan_compute_queue_family = Some(dev.compute_family);
                caps.vulkan_max_compute_invocations = dev.max_invocations;
                caps.has_vk_ahb = dev.has_ahb;
                caps.has_vk_dmabuf = dev.has_dmabuf;
            }
        }
    }

    pub fn probe_capabilities() -> GpuCapabilities {
        let mut caps = GpuCapabilities::default();
        let mut egl = AndroidGpuRenderer::new();
        egl.fill_egl_capabilities(&mut caps);
        fill_vulkan_capabilities(&mut caps);
        caps
    }

    pub fn compute_smoke() -> ComputeSmokeResult {
        runtime().compute_smoke()
    }

    pub(crate) unsafe fn run_vulkan_compute_bytes(
        spirv: &[u32],
        input: &[u8],
        output_len: usize,
        workgroups: [u32; 3],
        probe_ahb: bool,
    ) -> Result<(String, Vec<u8>, bool), String> {
        validate_spirv_structure(spirv)?;
        unsafe {
            let vk = VkLoader::new()?;
            let dev_info = select_compute_device(&vk)?;
            let get_mem_props: FnVkGetPhysicalDeviceMemoryProperties =
                vk.inst_fn(c"vkGetPhysicalDeviceMemoryProperties")?;
            let create_device: FnVkCreateDevice = vk.inst_fn(c"vkCreateDevice")?;
            let get_dev_proc: FnVkGetDeviceProcAddr = vk.inst_fn(c"vkGetDeviceProcAddr")?;

            let mut mem_props: VkPhysicalDeviceMemoryProperties = std::mem::zeroed();
            get_mem_props(dev_info.phys, &mut mem_props);

            let prio = 1.0f32;
            let qci = VkDeviceQueueCreateInfo {
                s_type: VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO,
                p_next: ptr::null(),
                flags: 0,
                queue_family_index: dev_info.compute_family,
                queue_count: 1,
                p_queue_priorities: &prio,
            };
            let ext_ahb = c"VK_ANDROID_external_memory_android_hardware_buffer".as_ptr();
            let dci = VkDeviceCreateInfo {
                s_type: VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO,
                p_next: ptr::null(),
                flags: 0,
                queue_create_info_count: 1,
                p_queue_create_infos: &qci,
                enabled_layer_count: 0,
                pp_enabled_layer_names: ptr::null(),
                enabled_extension_count: if dev_info.has_ahb { 1 } else { 0 },
                pp_enabled_extension_names: if dev_info.has_ahb {
                    &ext_ahb
                } else {
                    ptr::null()
                },
                p_enabled_features: ptr::null(),
            };
            let mut device: VkDevice = ptr::null_mut();
            let rc = create_device(dev_info.phys, &dci, ptr::null(), &mut device);
            if rc != VK_SUCCESS || device.is_null() {
                return Err(format!("vkCreateDevice failed ({rc})"));
            }

            macro_rules! dev_fn {
                ($ty:ty, $cname:expr) => {{
                    let p = get_dev_proc(device, $cname.as_ptr());
                    if p.is_null() {
                        let d: FnVkDestroyDevice =
                            std::mem::transmute(get_dev_proc(device, c"vkDestroyDevice".as_ptr()));
                        d(device, ptr::null());
                        return Err(format!("missing device fn {}", $cname.to_string_lossy()));
                    }
                    std::mem::transmute::<*const c_void, $ty>(p)
                }};
            }

            let destroy_device = dev_fn!(FnVkDestroyDevice, c"vkDestroyDevice");
            let get_queue = dev_fn!(FnVkGetDeviceQueue, c"vkGetDeviceQueue");
            let create_buffer = dev_fn!(FnVkCreateBuffer, c"vkCreateBuffer");
            let destroy_buffer = dev_fn!(FnVkDestroyBuffer, c"vkDestroyBuffer");
            let get_buf_reqs = dev_fn!(
                FnVkGetBufferMemoryRequirements,
                c"vkGetBufferMemoryRequirements"
            );
            let alloc_mem = dev_fn!(FnVkAllocateMemory, c"vkAllocateMemory");
            let free_mem = dev_fn!(FnVkFreeMemory, c"vkFreeMemory");
            let bind_buf_mem = dev_fn!(FnVkBindBufferMemory, c"vkBindBufferMemory");
            let map_mem = dev_fn!(FnVkMapMemory, c"vkMapMemory");
            let unmap_mem = dev_fn!(FnVkUnmapMemory, c"vkUnmapMemory");
            let create_shader = dev_fn!(FnVkCreateShaderModule, c"vkCreateShaderModule");
            let destroy_shader = dev_fn!(FnVkDestroyShaderModule, c"vkDestroyShaderModule");
            let create_dsl = dev_fn!(
                FnVkCreateDescriptorSetLayout,
                c"vkCreateDescriptorSetLayout"
            );
            let destroy_dsl = dev_fn!(
                FnVkDestroyDescriptorSetLayout,
                c"vkDestroyDescriptorSetLayout"
            );
            let create_pl = dev_fn!(FnVkCreatePipelineLayout, c"vkCreatePipelineLayout");
            let destroy_pl = dev_fn!(FnVkDestroyPipelineLayout, c"vkDestroyPipelineLayout");
            let create_cp = dev_fn!(FnVkCreateComputePipelines, c"vkCreateComputePipelines");
            let destroy_pipe = dev_fn!(FnVkDestroyPipeline, c"vkDestroyPipeline");
            let create_dp = dev_fn!(FnVkCreateDescriptorPool, c"vkCreateDescriptorPool");
            let destroy_dp = dev_fn!(FnVkDestroyDescriptorPool, c"vkDestroyDescriptorPool");
            let alloc_ds = dev_fn!(FnVkAllocateDescriptorSets, c"vkAllocateDescriptorSets");
            let update_ds = dev_fn!(FnVkUpdateDescriptorSets, c"vkUpdateDescriptorSets");
            let create_cmd_pool = dev_fn!(FnVkCreateCommandPool, c"vkCreateCommandPool");
            let destroy_cmd_pool = dev_fn!(FnVkDestroyCommandPool, c"vkDestroyCommandPool");
            let alloc_cmd = dev_fn!(FnVkAllocateCommandBuffers, c"vkAllocateCommandBuffers");
            let begin_cmd = dev_fn!(FnVkBeginCommandBuffer, c"vkBeginCommandBuffer");
            let end_cmd = dev_fn!(FnVkEndCommandBuffer, c"vkEndCommandBuffer");
            let cmd_bind_pipe = dev_fn!(FnVkCmdBindPipeline, c"vkCmdBindPipeline");
            let cmd_bind_ds = dev_fn!(FnVkCmdBindDescriptorSets, c"vkCmdBindDescriptorSets");
            let cmd_dispatch = dev_fn!(FnVkCmdDispatch, c"vkCmdDispatch");
            let queue_submit = dev_fn!(FnVkQueueSubmit, c"vkQueueSubmit");
            let create_fence = dev_fn!(FnVkCreateFence, c"vkCreateFence");
            let destroy_fence = dev_fn!(FnVkDestroyFence, c"vkDestroyFence");
            let wait_fences = dev_fn!(FnVkWaitForFences, c"vkWaitForFences");

            let mut queue: VkQueue = ptr::null_mut();
            get_queue(device, dev_info.compute_family, 0, &mut queue);

            // Pad buffers to at least 256 bytes (64 f32 lanes) aligned to 4 bytes
            // so workgroups up to local_size_x=64 never access out of bounds.
            let in_size = (((input.len().max(256)) + 3) & !3) as VkDeviceSize;
            let out_size = (((output_len.max(256)) + 3) & !3) as VkDeviceSize;

            let make_buf = |byte_size: VkDeviceSize| -> Result<(VkBuffer, VkDeviceMemory), String> {
                let bci = VkBufferCreateInfo {
                    s_type: VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO,
                    p_next: ptr::null(),
                    flags: 0,
                    size: byte_size,
                    usage: VK_BUFFER_USAGE_STORAGE_BUFFER_BIT,
                    sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
                    queue_family_index_count: 0,
                    p_queue_family_indices: ptr::null(),
                };
                let mut buf: VkBuffer = 0;
                let rc = create_buffer(device, &bci, ptr::null(), &mut buf);
                if rc != VK_SUCCESS {
                    return Err(format!("vkCreateBuffer failed ({rc})"));
                }
                let mut reqs = VkMemoryRequirements::default();
                get_buf_reqs(device, buf, &mut reqs);
                let want =
                    VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT;
                let mem_idx = (0..mem_props.memory_type_count)
                    .find(|&idx| {
                        ((reqs.memory_type_bits & (1 << idx)) != 0)
                            && ((mem_props.memory_types[idx as usize].property_flags & want)
                                == want)
                    })
                    .ok_or_else(|| "no HOST_VISIBLE | HOST_COHERENT memory type".to_string())?;
                let mai = VkMemoryAllocateInfo {
                    s_type: VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
                    p_next: ptr::null(),
                    allocation_size: reqs.size,
                    memory_type_index: mem_idx,
                };
                let mut mem: VkDeviceMemory = 0;
                let rc = alloc_mem(device, &mai, ptr::null(), &mut mem);
                if rc != VK_SUCCESS {
                    destroy_buffer(device, buf, ptr::null());
                    return Err(format!("vkAllocateMemory failed ({rc})"));
                }
                let rc = bind_buf_mem(device, buf, mem, 0);
                if rc != VK_SUCCESS {
                    free_mem(device, mem, ptr::null());
                    destroy_buffer(device, buf, ptr::null());
                    return Err(format!("vkBindBufferMemory failed ({rc})"));
                }
                Ok((buf, mem))
            };

            let (in_buf, in_mem) = make_buf(in_size)?;
            let (out_buf, out_mem) = match make_buf(out_size) {
                Ok(pair) => pair,
                Err(e) => {
                    free_mem(device, in_mem, ptr::null());
                    destroy_buffer(device, in_buf, ptr::null());
                    destroy_device(device, ptr::null());
                    return Err(e);
                }
            };

            let mut mapped: *mut c_void = ptr::null_mut();
            let rc = map_mem(device, in_mem, 0, in_size, 0, &mut mapped);
            if rc != VK_SUCCESS || mapped.is_null() {
                free_mem(device, out_mem, ptr::null());
                destroy_buffer(device, out_buf, ptr::null());
                free_mem(device, in_mem, ptr::null());
                destroy_buffer(device, in_buf, ptr::null());
                destroy_device(device, ptr::null());
                return Err(format!("vkMapMemory(in) failed ({rc})"));
            }
            ptr::write_bytes(mapped as *mut u8, 0, in_size as usize);
            if !input.is_empty() {
                ptr::copy_nonoverlapping(input.as_ptr(), mapped as *mut u8, input.len());
            }
            unmap_mem(device, in_mem);

            let smci = VkShaderModuleCreateInfo {
                s_type: VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO,
                p_next: ptr::null(),
                flags: 0,
                code_size: std::mem::size_of_val(spirv),
                p_code: spirv.as_ptr(),
            };
            let mut shader: VkShaderModule = 0;
            let rc = create_shader(device, &smci, ptr::null(), &mut shader);

            let bindings = [
                VkDescriptorSetLayoutBinding {
                    binding: 0,
                    descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
                    descriptor_count: 1,
                    stage_flags: VK_SHADER_STAGE_COMPUTE_BIT,
                    p_immutable_samplers: ptr::null(),
                },
                VkDescriptorSetLayoutBinding {
                    binding: 1,
                    descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
                    descriptor_count: 1,
                    stage_flags: VK_SHADER_STAGE_COMPUTE_BIT,
                    p_immutable_samplers: ptr::null(),
                },
            ];
            let dsl_ci = VkDescriptorSetLayoutCreateInfo {
                s_type: VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO,
                p_next: ptr::null(),
                flags: 0,
                binding_count: 2,
                p_bindings: bindings.as_ptr(),
            };
            let mut dsl: VkDescriptorSetLayout = 0;
            let rc_dsl = if rc == VK_SUCCESS {
                create_dsl(device, &dsl_ci, ptr::null(), &mut dsl)
            } else {
                rc
            };

            let pl_ci = VkPipelineLayoutCreateInfo {
                s_type: VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO,
                p_next: ptr::null(),
                flags: 0,
                set_layout_count: 1,
                p_set_layouts: &dsl,
                push_constant_range_count: 0,
                p_push_constant_ranges: ptr::null(),
            };
            let mut pl: VkPipelineLayout = 0;
            let rc_pl = if rc_dsl == VK_SUCCESS {
                create_pl(device, &pl_ci, ptr::null(), &mut pl)
            } else {
                rc_dsl
            };

            let cp_ci = VkComputePipelineCreateInfo {
                s_type: VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO,
                p_next: ptr::null(),
                flags: 0,
                stage: VkPipelineShaderStageCreateInfo {
                    s_type: VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
                    p_next: ptr::null(),
                    flags: 0,
                    stage: VK_SHADER_STAGE_COMPUTE_BIT,
                    module: shader,
                    p_name: c"main".as_ptr(),
                    p_specialization_info: ptr::null(),
                },
                layout: pl,
                base_pipeline_handle: 0,
                base_pipeline_index: -1,
            };
            let mut pipeline: VkPipeline = 0;
            let rc_cp = if rc_pl == VK_SUCCESS {
                create_cp(device, 0, 1, &cp_ci, ptr::null(), &mut pipeline)
            } else {
                rc_pl
            };

            let pool_size = VkDescriptorPoolSize {
                descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
                descriptor_count: 2,
            };
            let dp_ci = VkDescriptorPoolCreateInfo {
                s_type: VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO,
                p_next: ptr::null(),
                flags: 0,
                max_sets: 1,
                pool_size_count: 1,
                p_pool_sizes: &pool_size,
            };
            let mut dpool: VkDescriptorPool = 0;
            let mut dset: VkDescriptorSet = 0;
            let rc_ds = if rc_cp == VK_SUCCESS {
                let r = create_dp(device, &dp_ci, ptr::null(), &mut dpool);
                if r == VK_SUCCESS {
                    let ds_ai = VkDescriptorSetAllocateInfo {
                        s_type: VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO,
                        p_next: ptr::null(),
                        descriptor_pool: dpool,
                        descriptor_set_count: 1,
                        p_set_layouts: &dsl,
                    };
                    alloc_ds(device, &ds_ai, &mut dset)
                } else {
                    r
                }
            } else {
                rc_cp
            };

            if rc_ds == VK_SUCCESS {
                let bi_in = VkDescriptorBufferInfo {
                    buffer: in_buf,
                    offset: 0,
                    range: in_size,
                };
                let bi_out = VkDescriptorBufferInfo {
                    buffer: out_buf,
                    offset: 0,
                    range: out_size,
                };
                let writes = [
                    VkWriteDescriptorSet {
                        s_type: VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET,
                        p_next: ptr::null(),
                        dst_set: dset,
                        dst_binding: 0,
                        dst_array_element: 0,
                        descriptor_count: 1,
                        descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
                        p_image_info: ptr::null(),
                        p_buffer_info: &bi_in,
                        p_texel_buffer_view: ptr::null(),
                    },
                    VkWriteDescriptorSet {
                        s_type: VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET,
                        p_next: ptr::null(),
                        dst_set: dset,
                        dst_binding: 1,
                        dst_array_element: 0,
                        descriptor_count: 1,
                        descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
                        p_image_info: ptr::null(),
                        p_buffer_info: &bi_out,
                        p_texel_buffer_view: ptr::null(),
                    },
                ];
                update_ds(device, 2, writes.as_ptr(), 0, ptr::null());
            }

            let cpool_ci = VkCommandPoolCreateInfo {
                s_type: VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO,
                p_next: ptr::null(),
                flags: 0,
                queue_family_index: dev_info.compute_family,
            };
            let mut cpool: VkCommandPool = 0;
            let mut cmd: VkCommandBuffer = ptr::null_mut();
            let mut fence: VkFence = 0;
            let rc_exec = if rc_ds == VK_SUCCESS {
                let r = create_cmd_pool(device, &cpool_ci, ptr::null(), &mut cpool);
                if r == VK_SUCCESS {
                    let cb_ai = VkCommandBufferAllocateInfo {
                        s_type: VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO,
                        p_next: ptr::null(),
                        command_pool: cpool,
                        level: VK_COMMAND_BUFFER_LEVEL_PRIMARY,
                        command_buffer_count: 1,
                    };
                    let r2 = alloc_cmd(device, &cb_ai, &mut cmd);
                    if r2 == VK_SUCCESS {
                        let begin = VkCommandBufferBeginInfo {
                            s_type: VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
                            p_next: ptr::null(),
                            flags: VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
                            p_inheritance_info: ptr::null(),
                        };
                        let _ = begin_cmd(cmd, &begin);
                        cmd_bind_pipe(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, pipeline);
                        cmd_bind_ds(
                            cmd,
                            VK_PIPELINE_BIND_POINT_COMPUTE,
                            pl,
                            0,
                            1,
                            &dset,
                            0,
                            ptr::null(),
                        );
                        cmd_dispatch(
                            cmd,
                            workgroups[0].max(1),
                            workgroups[1].max(1),
                            workgroups[2].max(1),
                        );
                        let _ = end_cmd(cmd);
                        let fci = VkFenceCreateInfo {
                            s_type: VK_STRUCTURE_TYPE_FENCE_CREATE_INFO,
                            p_next: ptr::null(),
                            flags: 0,
                        };
                        let r3 = create_fence(device, &fci, ptr::null(), &mut fence);
                        if r3 == VK_SUCCESS {
                            let submit = VkSubmitInfo {
                                s_type: VK_STRUCTURE_TYPE_SUBMIT_INFO,
                                p_next: ptr::null(),
                                wait_semaphore_count: 0,
                                p_wait_semaphores: ptr::null(),
                                p_wait_dst_stage_mask: ptr::null(),
                                command_buffer_count: 1,
                                p_command_buffers: &cmd,
                                signal_semaphore_count: 0,
                                p_signal_semaphores: ptr::null(),
                            };
                            let r4 = queue_submit(queue, 1, &submit, fence);
                            if r4 == VK_SUCCESS {
                                wait_fences(device, 1, &fence, 1, 5_000_000_000)
                            } else {
                                r4
                            }
                        } else {
                            r3
                        }
                    } else {
                        r2
                    }
                } else {
                    r
                }
            } else {
                rc_ds
            };

            let mut out_vec = Vec::new();
            if rc_exec == VK_SUCCESS && output_len > 0 {
                let mut out_mapped: *mut c_void = ptr::null_mut();
                if map_mem(device, out_mem, 0, out_size, 0, &mut out_mapped) == VK_SUCCESS
                    && !out_mapped.is_null()
                {
                    out_vec =
                        std::slice::from_raw_parts(out_mapped as *const u8, output_len).to_vec();
                    unmap_mem(device, out_mem);
                }
            }

            // Probe AHardwareBuffer -> Vulkan properties when requested.
            let mut ahb_vulkan_ok = false;
            if probe_ahb && dev_info.has_ahb {
                let p_ahb = get_dev_proc(
                    device,
                    c"vkGetAndroidHardwareBufferPropertiesANDROID".as_ptr(),
                );
                if !p_ahb.is_null() {
                    let get_ahb_props: FnVkGetAndroidHardwareBufferPropertiesANDROID =
                        std::mem::transmute(p_ahb);
                    let desc = AHardwareBufferDesc {
                        width: 64,
                        height: 64,
                        layers: 1,
                        format: AHARDWAREBUFFER_FORMAT_R8G8B8A8_UNORM,
                        usage: AHARDWAREBUFFER_USAGE_GPU_SAMPLED_IMAGE
                            | AHARDWAREBUFFER_USAGE_GPU_COLOR_OUTPUT,
                        stride: 0,
                        rfu0: 0,
                        rfu1: 0,
                    };
                    let mut ahb: *mut AHardwareBuffer = ptr::null_mut();
                    if AHardwareBuffer_allocate(&desc, &mut ahb) == 0 && !ahb.is_null() {
                        let mut fmt_props: VkAndroidHardwareBufferFormatPropertiesANDROID =
                            std::mem::zeroed();
                        fmt_props.s_type =
                            VK_STRUCTURE_TYPE_ANDROID_HARDWARE_BUFFER_FORMAT_PROPERTIES_ANDROID;
                        let mut ahb_props: VkAndroidHardwareBufferPropertiesANDROID =
                            std::mem::zeroed();
                        ahb_props.s_type =
                            VK_STRUCTURE_TYPE_ANDROID_HARDWARE_BUFFER_PROPERTIES_ANDROID;
                        ahb_props.p_next = &mut fmt_props as *mut _ as *mut c_void;
                        if get_ahb_props(device, ahb, &mut ahb_props) == VK_SUCCESS
                            && ahb_props.allocation_size > 0
                            && ahb_props.memory_type_bits != 0
                        {
                            ahb_vulkan_ok = true;
                        }
                        AHardwareBuffer_release(ahb);
                    }
                }
            }

            if fence != 0 {
                destroy_fence(device, fence, ptr::null());
            }
            if cpool != 0 {
                destroy_cmd_pool(device, cpool, ptr::null());
            }
            if dpool != 0 {
                destroy_dp(device, dpool, ptr::null());
            }
            if pipeline != 0 {
                destroy_pipe(device, pipeline, ptr::null());
            }
            if pl != 0 {
                destroy_pl(device, pl, ptr::null());
            }
            if dsl != 0 {
                destroy_dsl(device, dsl, ptr::null());
            }
            if shader != 0 {
                destroy_shader(device, shader, ptr::null());
            }
            free_mem(device, out_mem, ptr::null());
            destroy_buffer(device, out_buf, ptr::null());
            free_mem(device, in_mem, ptr::null());
            destroy_buffer(device, in_buf, ptr::null());
            destroy_device(device, ptr::null());

            if rc_exec != VK_SUCCESS {
                return Err(format!("Vulkan compute execution failed (rc={rc_exec})"));
            }
            Ok((dev_info.name, out_vec, ahb_vulkan_ok))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spirv_compute_shader_is_structurally_valid() {
        let words = build_square_plus_one_spirv();
        validate_spirv_structure(&words).expect("valid SPIR-V 1.0 stream");
        assert_eq!(words[0], 0x0723_0203);
        assert_eq!(words[1], 0x0001_0000);
        // Verify every result ID in the stream stays strictly below header bound.
        let bound = words[3];
        assert!(bound > 20);
        assert!(words.len() > 40);
    }

    #[test]
    fn compute_smoke_evaluates_correctly() {
        let res = compute_smoke();
        assert!(res.ok && res.sample_ok, "{res:?}");
    }

    #[test]
    fn spirv_relu_shader_is_structurally_valid() {
        let words = build_relu_spirv();
        validate_spirv_structure(&words).expect("valid ReLU SPIR-V 1.0 stream");
        assert_eq!(words[0], 0x0723_0203);
        assert_eq!(words[1], 0x0001_0000);
        assert!(words[3] > 20);
    }

    #[test]
    fn runtime_dispatch_f32_kernels_and_spirv_validation() {
        let rt = runtime();
        let (sq_res, sq_out) = rt.dispatch_f32_kernel("square_plus_one", &[-2.0, 0.0, 3.0]);
        assert!(sq_res.ok, "{sq_res:?}");
        assert_eq!(sq_out, vec![5.0, 1.0, 10.0]);

        let (relu_res, relu_out) = rt.dispatch_f32_kernel("relu", &[-3.5, 0.0, 2.25]);
        assert!(relu_res.ok, "{relu_res:?}");
        assert_eq!(relu_out, vec![0.0, 0.0, 2.25]);

        let (bad_kernel, bad_out) = rt.dispatch_f32_kernel("nope", &[1.0]);
        assert!(!bad_kernel.ok);
        assert!(bad_out.is_empty());

        let (bad_spirv, _) = rt.dispatch_spirv(&[1, 2, 3], &[0; 16], 16, [1, 1, 1]);
        assert!(!bad_spirv.ok);

        let valid_bytes: Vec<u8> = build_relu_spirv()
            .into_iter()
            .flat_map(|w| w.to_le_bytes())
            .collect();
        let (zero_wg, _) = rt.dispatch_spirv(&valid_bytes, &[0; 16], 16, [0, 1, 1]);
        assert!(!zero_wg.ok);
    }

    #[test]
    fn cpu_blit_rgba_swizzles_crops_and_respects_stride() {
        // 2x2 BGRA image (WL_SHM_FORMAT_ARGB8888):
        // top-left:  B=10, G=20, R=30, A=40
        // top-right: B=50, G=60, R=70, A=80
        // bot-left:  B=90, G=100, R=110, A=120
        // bot-right: B=130, G=140, R=150, A=160
        let src_pixels = vec![
            10, 20, 30, 40, 50, 60, 70, 80, // row 0
            90, 100, 110, 120, 130, 140, 150, 160, // row 1
        ];
        // Stride = 3 pixels (12 bytes per row) on 2x2 destination, crop right half [0.5, 0.0, 1.0, 1.0].
        let mut dst = vec![0u8; 3 * 2 * 4];
        cpu_blit_rgba(
            &mut dst,
            2,
            2,
            3,
            ShmFrameRef {
                width: 2,
                height: 2,
                stride: 8,
                wl_format: WL_SHM_FORMAT_ARGB8888,
                pixels: &src_pixels,
            },
            [0.5, 0.0, 1.0, 1.0],
        )
        .expect("blit BGRA right half");
        // Both dx=0 and dx=1 sample column 1 in the right half -> [R, G, B, A] = [70, 60, 50, 80] and [150, 140, 130, 160].
        assert_eq!(&dst[0..4], &[70, 60, 50, 80]);
        assert_eq!(&dst[4..8], &[70, 60, 50, 80]);
        assert_eq!(&dst[8..12], &[0, 0, 0, 0]); // padding pixel untouched
        assert_eq!(&dst[12..16], &[150, 140, 130, 160]);

        // Verify XRGB forces alpha = 255.
        let mut dst_xrgb = vec![0u8; 2 * 2 * 4];
        cpu_blit_rgba(
            &mut dst_xrgb,
            2,
            2,
            2,
            ShmFrameRef {
                width: 2,
                height: 2,
                stride: 8,
                wl_format: WL_SHM_FORMAT_XRGB8888,
                pixels: &src_pixels,
            },
            [0.0, 0.0, 1.0, 1.0],
        )
        .expect("blit XRGB");
        assert_eq!(&dst_xrgb[0..4], &[30, 20, 10, 255]);
    }

    #[test]
    fn cpu_fill_placeholder_draws_border_and_interior() {
        let mut dst = vec![0u8; 8 * 8 * 4];
        cpu_fill_placeholder(&mut dst, 8, 8, 8);
        // Border pixel (0, 0)
        assert_eq!(&dst[0..4], &[46, 64, 82, 255]);
        // Interior pixel (4, 4)
        let mid = (4 * 8 + 4) * 4;
        assert_eq!(&dst[mid..mid + 4], &[20, 26, 31, 255]);
    }

    #[test]
    fn force_cpu_switches_capabilities_and_compute_to_cpu() {
        let rt = runtime();
        rt.set_force_cpu(true);
        let caps = rt.capabilities();
        assert!(caps.force_cpu);
        assert!(caps.cpu_fallback_active);
        let (res, out) = rt.dispatch_f32_kernel("relu", &[-1.0, 2.0]);
        assert!(
            res.ok && !res.used_gpu && res.backend == "cpu-fallback",
            "{res:?}"
        );
        assert_eq!(out, vec![0.0, 2.0]);
        rt.set_force_cpu(false);
        assert!(!rt.capabilities().force_cpu);
    }
}
