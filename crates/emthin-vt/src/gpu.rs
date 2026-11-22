//! GLES 3 backend for the retained renderer.
//!
//! The CPU atlas and rows are the source of truth. Changed rows upload their
//! quads; unchanged rows keep their buffers. A context loss drops only the
//! GPU objects, and re-creating this type restores them from the CPU state.

use std::ffi::c_void;

use glow::HasContext;
use khronos_egl as egl;

use crate::render::{Color, Image, Rect, Renderer, Row};

const PLATFORM_SURFACELESS_MESA: egl::Enum = 0x31dd;
const SURFACE_WIDTH: i32 = 4096;
const SURFACE_HEIGHT: i32 = 4096;

#[derive(Debug)]
pub struct GpuError(pub String);

impl std::fmt::Display for GpuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for GpuError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameStats {
    pub uploaded_atlas: bool,
    pub rows_updated: usize,
    pub quads: usize,
    pub width: u16,
    pub height: u16,
}

/// One retained row, with separate buffers for its solid and glyph quads.
struct RowGpu {
    row: Row,
    solid_vao: glow::VertexArray,
    solid_vbo: glow::Buffer,
    solid_count: i32,
    glyph_vao: glow::VertexArray,
    glyph_vbo: glow::Buffer,
    glyph_count: i32,
}

impl RowGpu {
    unsafe fn new(gl: &glow::Context) -> Result<Self, GpuError> {
        Ok(Self {
            row: Row::default(),
            solid_vao: create_vao(gl)?,
            solid_vbo: create_vbo(gl)?,
            solid_count: 0,
            glyph_vao: create_vao(gl)?,
            glyph_vbo: create_vbo(gl)?,
            glyph_count: 0,
        })
    }

    unsafe fn upload(
        &mut self,
        gl: &glow::Context,
        scratch: &mut Vec<f32>,
        row: &Row,
        atlas: (u16, u16),
    ) {
        scratch.clear();
        for solid in &row.solids {
            push_rect(scratch, solid.rect, solid.color);
        }
        self.solid_count = (scratch.len() / 7) as i32;
        gl.bind_vertex_array(Some(self.solid_vao));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.solid_vbo));
        set_layout(gl);
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, as_bytes(scratch), glow::DYNAMIC_DRAW);

        scratch.clear();
        for glyph in &row.glyphs {
            push_glyph(scratch, glyph.rect, glyph.uv, glyph.color, atlas);
        }
        self.glyph_count = (scratch.len() / 7) as i32;
        gl.bind_vertex_array(Some(self.glyph_vao));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.glyph_vbo));
        set_layout(gl);
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, as_bytes(scratch), glow::DYNAMIC_DRAW);
        self.row = row.clone();
    }

    unsafe fn delete(&self, gl: &glow::Context) {
        gl.delete_vertex_array(self.solid_vao);
        gl.delete_buffer(self.solid_vbo);
        gl.delete_vertex_array(self.glyph_vao);
        gl.delete_buffer(self.glyph_vbo);
    }
}

pub struct GlRenderer {
    egl: egl::DynamicInstance<egl::EGL1_5>,
    display: egl::Display,
    context: egl::Context,
    surface: egl::Surface,
    gl: glow::Context,
    solid_program: glow::Program,
    glyph_program: glow::Program,
    texture: glow::Texture,
    atlas_generation: Option<u64>,
    rows: Vec<RowGpu>,
    cursor: Option<(RowGpu, Rect, Color)>,
    scratch: Vec<f32>,
}

impl GlRenderer {
    /// Creates an offscreen GLES 3 surface through EGL's surfaceless platform.
    pub fn new() -> Result<Self, GpuError> {
        unsafe {
            let egl = egl::DynamicInstance::<egl::EGL1_5>::load_required()
                .map_err(|error| GpuError(format!("load libEGL: {error}")))?;
            let display = egl
                .get_platform_display(
                    PLATFORM_SURFACELESS_MESA,
                    egl::DEFAULT_DISPLAY,
                    &[egl::ATTRIB_NONE],
                )
                .map_err(|error| GpuError(format!("surfaceless EGL display: {error}")))?;
            egl.initialize(display)
                .map_err(|error| GpuError(format!("initialize EGL: {error}")))?;
            let config = egl
                .choose_first_config(
                    display,
                    &[
                        egl::RENDERABLE_TYPE,
                        egl::OPENGL_ES3_BIT,
                        egl::SURFACE_TYPE,
                        egl::PBUFFER_BIT,
                        egl::RED_SIZE,
                        8,
                        egl::GREEN_SIZE,
                        8,
                        egl::BLUE_SIZE,
                        8,
                        egl::ALPHA_SIZE,
                        8,
                        egl::NONE,
                    ],
                )
                .map_err(|error| GpuError(format!("choose EGL config: {error}")))?
                .ok_or_else(|| GpuError("no GLES 3 pbuffer config".into()))?;
            let context = egl
                .create_context(
                    display,
                    config,
                    None,
                    &[
                        egl::CONTEXT_MAJOR_VERSION,
                        3,
                        egl::CONTEXT_MINOR_VERSION,
                        0,
                        egl::NONE,
                    ],
                )
                .map_err(|error| GpuError(format!("create EGL context: {error}")))?;
            let surface = egl
                .create_pbuffer_surface(
                    display,
                    config,
                    &[
                        egl::WIDTH,
                        SURFACE_WIDTH,
                        egl::HEIGHT,
                        SURFACE_HEIGHT,
                        egl::NONE,
                    ],
                )
                .map_err(|error| GpuError(format!("create EGL pbuffer: {error}")))?;
            egl.make_current(display, Some(surface), Some(surface), Some(context))
                .map_err(|error| GpuError(format!("make EGL current: {error}")))?;

            let gl = glow::Context::from_loader_function(|name| {
                egl.get_proc_address(name)
                    .map(|function| function as *const c_void)
                    .unwrap_or(std::ptr::null())
            });

            let solid_program = program(
                &gl,
                SOLID_VERTEX,
                "#version 300 es\nprecision mediump float;\nin vec4 v_color;\nout vec4 out_color;\nvoid main(){out_color=v_color;}\n",
            )?;
            let glyph_program = program(&gl, TEXTURE_VERTEX, TEXTURE_FRAGMENT)?;
            let texture = gl
                .create_texture()
                .map_err(|error| GpuError(format!("create texture: {error}")))?;
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::NEAREST as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::NEAREST as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);

            Ok(Self {
                egl,
                display,
                context,
                surface,
                gl,
                solid_program,
                glyph_program,
                texture,
                atlas_generation: None,
                rows: Vec::new(),
                cursor: None,
                scratch: Vec::new(),
            })
        }
    }

    /// Draws retained batches and reads the result back top-down.
    pub fn render(
        &mut self,
        renderer: &Renderer,
        background: Color,
    ) -> Result<(Image, FrameStats), GpuError> {
        let width = renderer
            .metrics()
            .cell_width
            .saturating_mul(renderer.cols());
        let height = (renderer.rows().len() as u16).saturating_mul(renderer.metrics().cell_height);
        if width == 0 || height == 0 {
            return Ok((
                Image {
                    width,
                    height,
                    pixels: Vec::new(),
                },
                FrameStats {
                    uploaded_atlas: false,
                    rows_updated: 0,
                    quads: 0,
                    width,
                    height,
                },
            ));
        }

        unsafe {
            let gl = &self.gl;
            gl.viewport(0, 0, width as i32, height as i32);
            gl.disable(glow::DEPTH_TEST);
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
            gl.clear_color(
                f32::from(background.0[0]) / 255.0,
                f32::from(background.0[1]) / 255.0,
                f32::from(background.0[2]) / 255.0,
                1.0,
            );
            gl.clear(glow::COLOR_BUFFER_BIT);

            let mut uploaded_atlas = false;
            let mut atlas_size = (1, 1);
            if self.atlas_generation != Some(renderer.atlas().generation()) {
                if let Some((aw, ah, pixels)) = renderer.atlas().rgba() {
                    gl.active_texture(glow::TEXTURE0);
                    gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
                    gl.tex_image_2d(
                        glow::TEXTURE_2D,
                        0,
                        glow::RGBA8 as i32,
                        aw as i32,
                        ah as i32,
                        0,
                        glow::RGBA,
                        glow::UNSIGNED_BYTE,
                        glow::PixelUnpackData::Slice(Some(&pixels)),
                    );
                    atlas_size = (aw, ah);
                    uploaded_atlas = true;
                }
                self.atlas_generation = Some(renderer.atlas().generation());
            } else if let Some(size) = renderer.atlas().size() {
                atlas_size = size;
            }

            let mut rows_updated = 0;
            while self.rows.len() < renderer.rows().len() {
                self.rows.push(RowGpu::new(gl)?);
            }
            while self.rows.len() > renderer.rows().len() {
                if let Some(row) = self.rows.pop() {
                    row.delete(gl);
                }
            }
            let mut scratch = std::mem::take(&mut self.scratch);
            for (gpu, row) in self.rows.iter_mut().zip(renderer.rows()) {
                if gpu.row != *row {
                    gpu.upload(gl, &mut scratch, row, atlas_size);
                    rows_updated += 1;
                }
            }
            self.scratch = scratch;

            let cursor = renderer.cursor();
            let cached_cursor = self
                .cursor
                .as_ref()
                .and_then(|(gpu, _, _)| gpu.row.solids.first().copied());
            if cached_cursor != cursor {
                match (cursor, self.cursor.take()) {
                    (Some(solid), Some((mut gpu, _, _))) => {
                        let row = Row {
                            solids: vec![solid],
                            glyphs: Vec::new(),
                        };
                        gpu.upload(gl, &mut self.scratch, &row, atlas_size);
                        self.cursor = Some((gpu, solid.rect, solid.color));
                    }
                    (Some(solid), None) => {
                        let mut gpu = RowGpu::new(gl)?;
                        let row = Row {
                            solids: vec![solid],
                            glyphs: Vec::new(),
                        };
                        gpu.upload(gl, &mut self.scratch, &row, atlas_size);
                        self.cursor = Some((gpu, solid.rect, solid.color));
                    }
                    (None, Some((gpu, _, _))) => gpu.delete(gl),
                    (None, None) => {}
                }
            }

            gl.use_program(Some(self.solid_program));
            set_size(gl, self.solid_program, width, height);
            let mut quads = 0;
            for row in &self.rows {
                if row.solid_count > 0 {
                    gl.bind_vertex_array(Some(row.solid_vao));
                    gl.draw_arrays(glow::TRIANGLES, 0, row.solid_count);
                    quads += (row.solid_count / 6) as usize;
                }
            }
            if let Some((gpu, _, _)) = &self.cursor {
                if gpu.solid_count > 0 {
                    gl.bind_vertex_array(Some(gpu.solid_vao));
                    gl.draw_arrays(glow::TRIANGLES, 0, gpu.solid_count);
                    quads += 1;
                }
            }

            gl.use_program(Some(self.glyph_program));
            let location = gl.get_uniform_location(self.glyph_program, "u_texture");
            gl.uniform_1_i32(location.as_ref(), 0);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
            set_size(gl, self.glyph_program, width, height);
            for row in &self.rows {
                if row.glyph_count > 0 {
                    gl.bind_vertex_array(Some(row.glyph_vao));
                    gl.draw_arrays(glow::TRIANGLES, 0, row.glyph_count);
                    quads += (row.glyph_count / 6) as usize;
                }
            }

            let mut pixels = vec![0u8; width as usize * height as usize * 4];
            gl.read_pixels(
                0,
                0,
                width as i32,
                height as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixels)),
            );
            // GL rows start at the bottom; the retained image is top-down.
            let stride = width as usize * 4;
            for row in 0..height as usize / 2 {
                let top = row * stride;
                let bottom = (height as usize - 1 - row) * stride;
                for byte in 0..stride {
                    pixels.swap(top + byte, bottom + byte);
                }
            }
            Ok((
                Image {
                    width,
                    height,
                    pixels,
                },
                FrameStats {
                    uploaded_atlas,
                    rows_updated,
                    quads,
                    width,
                    height,
                },
            ))
        }
    }
}

impl Drop for GlRenderer {
    fn drop(&mut self) {
        unsafe {
            let gl = &self.gl;
            for row in &self.rows {
                row.delete(gl);
            }
            if let Some((row, _, _)) = &self.cursor {
                row.delete(gl);
            }
            gl.delete_program(self.solid_program);
            gl.delete_program(self.glyph_program);
            gl.delete_texture(self.texture);
            let _ = self.egl.make_current(self.display, None, None, None);
            let _ = self.egl.destroy_surface(self.display, self.surface);
            let _ = self.egl.destroy_context(self.display, self.context);
            let _ = self.egl.terminate(self.display);
        }
    }
}

const SOLID_VERTEX: &str = "#version 300 es\nprecision mediump float;\nlayout(location=0) in vec2 a_pos;\nlayout(location=1) in vec2 a_uv;\nlayout(location=2) in vec4 a_color;\nout vec4 v_color;\nuniform vec2 u_size;\nvoid main(){vec2 p=vec2(a_pos.x/u_size.x*2.0-1.0, 1.0-a_pos.y/u_size.y*2.0);gl_Position=vec4(p,0.0,1.0);v_color=a_color;}\n";
const TEXTURE_VERTEX: &str = "#version 300 es\nprecision mediump float;\nlayout(location=0) in vec2 a_pos;\nlayout(location=1) in vec2 a_uv;\nlayout(location=2) in vec4 a_color;\nout vec2 v_uv;\nout vec4 v_color;\nuniform vec2 u_size;\nvoid main(){vec2 p=vec2(a_pos.x/u_size.x*2.0-1.0, 1.0-a_pos.y/u_size.y*2.0);gl_Position=vec4(p,0.0,1.0);v_uv=a_uv;v_color=a_color;}\n";
const TEXTURE_FRAGMENT: &str = "#version 300 es\nprecision mediump float;\nuniform sampler2D u_texture;\nin vec2 v_uv;\nin vec4 v_color;\nout vec4 out_color;\nvoid main(){float a=texture(u_texture,v_uv).a;out_color=vec4(v_color.rgb,v_color.a*a);}\n";

unsafe fn program(
    gl: &glow::Context,
    vertex: &str,
    fragment: &str,
) -> Result<glow::Program, GpuError> {
    let program = gl
        .create_program()
        .map_err(|error| GpuError(format!("create program: {error}")))?;
    let mut shaders = Vec::new();
    for (kind, source) in [
        (glow::VERTEX_SHADER, vertex),
        (glow::FRAGMENT_SHADER, fragment),
    ] {
        let shader = gl
            .create_shader(kind)
            .map_err(|error| GpuError(format!("create shader: {error}")))?;
        gl.shader_source(shader, source);
        gl.compile_shader(shader);
        if !gl.get_shader_compile_status(shader) {
            return Err(GpuError(gl.get_shader_info_log(shader)));
        }
        gl.attach_shader(program, shader);
        shaders.push(shader);
    }
    gl.link_program(program);
    if !gl.get_program_link_status(program) {
        return Err(GpuError(gl.get_program_info_log(program)));
    }
    for shader in shaders {
        gl.detach_shader(program, shader);
        gl.delete_shader(shader);
    }
    Ok(program)
}

unsafe fn create_vbo(gl: &glow::Context) -> Result<glow::Buffer, GpuError> {
    gl.create_buffer()
        .map_err(|error| GpuError(format!("create VBO: {error}")))
}

unsafe fn create_vao(gl: &glow::Context) -> Result<glow::VertexArray, GpuError> {
    gl.create_vertex_array()
        .map_err(|error| GpuError(format!("create VAO: {error}")))
}

/// Records the interleaved position/UV/color layout on the bound VAO/VBO.
unsafe fn set_layout(gl: &glow::Context) {
    let stride = 7 * std::mem::size_of::<f32>() as i32;
    for (index, (size, offset)) in [(2, 0), (2, 2 * 4), (4, 4 * 4)].into_iter().enumerate() {
        gl.enable_vertex_attrib_array(index as u32);
        gl.vertex_attrib_pointer_f32(index as u32, size, glow::FLOAT, false, stride, offset);
    }
}

unsafe fn set_size(gl: &glow::Context, program: glow::Program, width: u16, height: u16) {
    let location = gl.get_uniform_location(program, "u_size");
    gl.uniform_2_f32(location.as_ref(), f32::from(width), f32::from(height));
}

fn as_bytes(values: &[f32]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), std::mem::size_of_val(values))
    }
}

fn push_rect(vertices: &mut Vec<f32>, rect: Rect, color: Color) {
    let [r, g, b, a] = [
        f32::from(color.0[0]) / 255.0,
        f32::from(color.0[1]) / 255.0,
        f32::from(color.0[2]) / 255.0,
        f32::from(color.0[3]) / 255.0,
    ];
    let x0 = f32::from(rect.x);
    let y0 = f32::from(rect.y);
    let x1 = f32::from(rect.x.saturating_add(rect.w));
    let y1 = f32::from(rect.y.saturating_add(rect.h));
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    for [x, y] in [[x0, y0], [x1, y0], [x1, y1], [x0, y0], [x1, y1], [x0, y1]] {
        vertices.extend_from_slice(&[x, y, 0.0, 0.0, r, g, b, a]);
    }
}

fn push_glyph(vertices: &mut Vec<f32>, rect: Rect, uv: Rect, color: Color, atlas: (u16, u16)) {
    let [r, g, b, a] = [
        f32::from(color.0[0]) / 255.0,
        f32::from(color.0[1]) / 255.0,
        f32::from(color.0[2]) / 255.0,
        f32::from(color.0[3]) / 255.0,
    ];
    let x0 = f32::from(rect.x);
    let y0 = f32::from(rect.y);
    let x1 = f32::from(rect.x.saturating_add(rect.w));
    let y1 = f32::from(rect.y.saturating_add(rect.h));
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let u0 = f32::from(uv.x) / f32::from(atlas.0);
    let v0 = f32::from(uv.y) / f32::from(atlas.1);
    let u1 = f32::from(uv.x.saturating_add(uv.w)) / f32::from(atlas.0);
    let v1 = f32::from(uv.y.saturating_add(uv.h)) / f32::from(atlas.1);
    for [x, y, u, v] in [
        [x0, y0, u0, v0],
        [x1, y0, u1, v0],
        [x1, y1, u1, v1],
        [x0, y0, u0, v0],
        [x1, y1, u1, v1],
        [x0, y1, u0, v1],
    ] {
        vertices.extend_from_slice(&[x, y, u, v, r, g, b, a]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{Metrics, Renderer};
    use crate::{Damage, DejaVuFont, Vt};

    fn fixture(text: &[u8], cols: u16) -> Renderer {
        let mut vt = Vt::new(cols, 1);
        vt.feed(text);
        let mut font = DejaVuFont::new();
        let mut renderer = Renderer::new(
            Metrics {
                cell_width: 8,
                cell_height: 16,
                baseline: 3,
            },
            64 * 1024,
            128,
        );
        renderer.update(&vt.screen(), &[], &mut font);
        renderer
    }

    #[test]
    fn gles_uploads_once_draws_and_survives_context_recreation() {
        let renderer = fixture(b"A", 4);
        let background = Color([0, 0, 0, 255]);

        let mut first = GlRenderer::new().expect("surfaceless GLES 3 context");
        let (image, stats) = first.render(&renderer, background).expect("first frame");
        assert_eq!((image.width, image.height), (32, 16));
        assert!(stats.uploaded_atlas, "the first frame uploads the atlas");
        assert_eq!(stats.rows_updated, 1);
        assert!(stats.quads >= 1);
        assert!(
            image
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| *pixel != background.0),
            "the GL frame contains glyph coverage"
        );
        let golden = image.checksum();

        let (again, stats) = first.render(&renderer, background).expect("repeat frame");
        assert!(
            !stats.uploaded_atlas,
            "unchanged rows re-upload no atlas page"
        );
        assert_eq!(stats.rows_updated, 0, "unchanged rows rebuild no geometry");
        assert_eq!(again.checksum(), golden, "repeat frame is deterministic");

        let mut vt = Vt::new(2, 2);
        vt.feed(b"B\r\nA");
        let mut font = DejaVuFont::new();
        let mut two_rows = Renderer::new(
            Metrics {
                cell_width: 8,
                cell_height: 16,
                baseline: 3,
            },
            64 * 1024,
            128,
        );
        two_rows.update(&vt.screen(), &[], &mut font);
        let stats = two_rows.update(&vt.screen(), &[], &mut font);
        assert_eq!(stats.rows_rebuilt, 0);
        first.render(&two_rows, background).expect("two-row frame");

        let mut changed = Vt::new(2, 2);
        changed.feed(b"B\r\nB");
        two_rows.update(
            &changed.screen(),
            &[Damage {
                x: 0,
                y: 1,
                cols: 1,
                rows: 1,
            }],
            &mut font,
        );
        let (_, stats) = first.render(&two_rows, background).expect("one-cell frame");
        assert!(!stats.uploaded_atlas, "the new glyph reuses an atlas slot");
        assert_eq!(stats.rows_updated, 1, "only the damaged row uploads");

        drop(first);
        let mut recreated = GlRenderer::new().expect("recreated context");
        let (after_loss, stats) = recreated
            .render(&renderer, background)
            .expect("frame after context loss");
        assert!(
            stats.uploaded_atlas,
            "re-created context restores the atlas"
        );
        assert_eq!(after_loss, image, "GPU pixels survive context recreation");
    }
}
