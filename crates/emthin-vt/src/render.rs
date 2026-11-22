//! Retained cell batches and a bounded glyph atlas.
//!
//! This is backend-neutral: a GL/GLES renderer consumes the quads and uploads
//! atlas pages. Unchanged rows keep their existing quads, a hidden pane does no
//! glyph work, and recreating GPU resources keeps the CPU atlas intact.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::{Damage, GlyphKey, Screen};

/// Cell geometry in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Metrics {
    pub cell_width: u16,
    pub cell_height: u16,
    pub baseline: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(pub [u8; 4]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Solid {
    pub rect: Rect,
    pub color: Color,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlyphDraw {
    pub rect: Rect,
    pub uv: Rect,
    pub color: Color,
    pub key: GlyphKey,
    pub pixel_size: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Glyph {
    pub width: u16,
    pub height: u16,
    pub advance: u16,
    /// Coverage bytes, one per pixel. The renderer owns alpha conversion.
    pub coverage: Vec<u8>,
}

/// Where glyph coverage comes from. Tests use deterministic blocks; a real
/// backend uses the font stack.
pub trait GlyphSource {
    fn glyph(&mut self, key: &GlyphKey, pixel_size: u16) -> Option<Glyph>;
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Row {
    pub solids: Vec<Solid>,
    pub glyphs: Vec<GlyphDraw>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stats {
    pub rows_rebuilt: usize,
    pub glyphs_rasterized: usize,
    pub quads: usize,
    pub hidden: bool,
}

#[derive(Debug, Clone)]
struct Slot {
    uv: Rect,
    advance: u16,
    coverage: Arc<Vec<u8>>,
}

pub struct Atlas {
    slots: BTreeMap<(GlyphKey, u16), Slot>,
    bytes: usize,
    budget: usize,
    next_x: u16,
    next_y: u16,
    row_height: u16,
    width: u16,
    generation: u64,
}

impl Atlas {
    pub fn new(budget: usize, width: u16) -> Self {
        Self {
            slots: BTreeMap::new(),
            bytes: 0,
            budget,
            next_x: 0,
            next_y: 0,
            row_height: 0,
            width: width.max(1),
            generation: 0,
        }
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn budget(&self) -> usize {
        self.budget
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn size(&self) -> Option<(u16, u16)> {
        (!self.slots.is_empty()).then(|| {
            (
                self.width,
                self.next_y.saturating_add(self.row_height).max(1),
            )
        })
    }

    /// Materializes the atlas as a white RGBA page whose alpha is coverage.
    /// Returns `None` while empty; the GL backend uploads this page.
    pub fn rgba(&self) -> Option<(u16, u16, Vec<u8>)> {
        let (width, height) = self.size()?;
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        for slot in self.slots.values() {
            for y in 0..slot.uv.h {
                for x in 0..slot.uv.w {
                    let alpha = slot.coverage[y as usize * slot.uv.w as usize + x as usize];
                    let px = slot.uv.x as usize + x as usize;
                    let py = slot.uv.y as usize + y as usize;
                    let i = (py * width as usize + px) * 4;
                    pixels[i..i + 4].copy_from_slice(&[255, 255, 255, alpha]);
                }
            }
        }
        Some((width, height, pixels))
    }

    pub fn slot(&self, key: &GlyphKey, size: u16) -> Option<Rect> {
        self.slots.get(&(key.clone(), size)).map(|slot| slot.uv)
    }

    fn get(&self, key: &GlyphKey, size: u16) -> Option<Slot> {
        self.slots.get(&(key.clone(), size)).cloned()
    }

    fn coverage(&self, key: &GlyphKey, size: u16) -> Option<&[u8]> {
        self.slots
            .get(&(key.clone(), size))
            .map(|slot| slot.coverage.as_slice())
    }

    fn insert(&mut self, key: GlyphKey, size: u16, glyph: &Glyph) -> Option<Slot> {
        let bytes = glyph.coverage.len().max(1);
        if self.bytes.saturating_add(bytes) > self.budget {
            return None;
        }
        if self.next_x.saturating_add(glyph.width) > self.width {
            self.next_x = 0;
            self.next_y = self.next_y.saturating_add(self.row_height);
            self.row_height = 0;
        }
        let uv = Rect {
            x: self.next_x,
            y: self.next_y,
            w: glyph.width,
            h: glyph.height,
        };
        self.next_x = self.next_x.saturating_add(glyph.width);
        self.row_height = self.row_height.max(glyph.height);
        self.bytes = self.bytes.saturating_add(bytes);
        let slot = Slot {
            uv,
            advance: glyph.advance,
            coverage: Arc::new(glyph.coverage.clone()),
        };
        self.generation = self.generation.saturating_add(1);
        self.slots.insert((key, size), slot.clone());
        Some(slot)
    }
}

/// Zoom is clamped so a pane cannot request a zero or unbounded cell size.
pub const MIN_ZOOM_PERCENT: u16 = 50;
pub const MAX_ZOOM_PERCENT: u16 = 300;

pub struct Renderer {
    base_metrics: Metrics,
    metrics: Metrics,
    zoom: u16,
    atlas: Atlas,
    rows: Vec<Row>,
    cols: u16,
    cursor: Option<Solid>,
    visible: bool,
    needs_full: bool,
    gl_generation: u64,
}

impl Renderer {
    pub fn new(metrics: Metrics, glyph_budget: usize, atlas_width: u16) -> Self {
        Self {
            base_metrics: metrics,
            metrics,
            zoom: 100,
            atlas: Atlas::new(glyph_budget, atlas_width),
            rows: Vec::new(),
            cols: 0,
            cursor: None,
            visible: true,
            needs_full: true,
            gl_generation: 0,
        }
    }

    pub fn atlas(&self) -> &Atlas {
        &self.atlas
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn cursor(&self) -> Option<Solid> {
        self.cursor
    }

    pub fn gl_generation(&self) -> u64 {
        self.gl_generation
    }

    pub fn set_visible(&mut self, visible: bool) {
        if !visible {
            self.visible = false;
        } else if !self.visible {
            self.visible = true;
            self.needs_full = true;
        }
    }

    /// GPU context loss invalidates uploaded pages but not the CPU atlas.
    pub fn recreate_gl(&mut self) -> u64 {
        self.gl_generation = self.gl_generation.saturating_add(1);
        self.gl_generation
    }

    pub fn metrics(&self) -> Metrics {
        self.metrics
    }

    pub fn zoom(&self) -> u16 {
        self.zoom
    }

    /// Set text zoom in percent, clamped to [`MIN_ZOOM_PERCENT`] and
    /// [`MAX_ZOOM_PERCENT`]. Returns the applied value.
    pub fn set_zoom(&mut self, percent: u16) -> u16 {
        let zoom = percent.clamp(MIN_ZOOM_PERCENT, MAX_ZOOM_PERCENT);
        if zoom != self.zoom {
            self.zoom = zoom;
            self.metrics = Metrics {
                cell_width: scale(self.base_metrics.cell_width, zoom),
                cell_height: scale(self.base_metrics.cell_height, zoom),
                baseline: scale(self.base_metrics.baseline, zoom),
            };
            self.needs_full = true;
        }
        zoom
    }

    /// Software composition of the retained batches, for tests and the debug
    /// view. A real backend uploads the same quads instead.
    pub fn compose(&self, background: Color) -> Image {
        let width = self.cols.saturating_mul(self.metrics.cell_width);
        let height = (self.rows.len() as u16).saturating_mul(self.metrics.cell_height);
        let mut image = Image {
            width,
            height,
            pixels: background.0.repeat(width as usize * height as usize),
        };
        for row in &self.rows {
            for solid in &row.solids {
                image.fill(solid.rect, solid.color);
            }
            for glyph in &row.glyphs {
                let Some(coverage) = self.atlas.coverage(&glyph.key, glyph.pixel_size) else {
                    continue;
                };
                image.blend_glyph(glyph, coverage);
            }
        }
        if let Some(cursor) = self.cursor {
            image.fill(cursor.rect, cursor.color);
        }
        image
    }

    pub fn update<S: GlyphSource>(
        &mut self,
        screen: &Screen,
        damage: &[Damage],
        source: &mut S,
    ) -> Stats {
        let mut stats = Stats::default();
        if !self.visible {
            stats.hidden = true;
            return stats;
        }
        if self.rows.len() != screen.rows as usize || self.cols != screen.cols {
            self.needs_full = true;
        }
        self.cols = screen.cols;
        let full = self.needs_full;
        self.needs_full = false;
        if full {
            self.rows = (0..screen.rows).map(|_| Row::default()).collect();
        }
        let damage = if full {
            vec![Damage {
                x: 0,
                y: 0,
                cols: screen.cols,
                rows: screen.rows,
            }]
        } else {
            damage.to_vec()
        };
        for rect in damage {
            for y in rect.y..rect.y.saturating_add(rect.rows).min(screen.rows) {
                let Some(row) = self.rows.get_mut(y as usize) else {
                    continue;
                };
                build_row(
                    row,
                    screen,
                    y,
                    rect.x,
                    rect.cols,
                    self.metrics,
                    &mut self.atlas,
                    source,
                    &mut stats,
                );
                stats.rows_rebuilt += 1;
            }
        }
        self.cursor = screen.cursor.visible.then(|| Solid {
            rect: Rect {
                x: screen.cursor.x.saturating_mul(self.metrics.cell_width),
                y: screen.cursor.y.saturating_mul(self.metrics.cell_height),
                w: self.metrics.cell_width.max(1),
                h: self.metrics.cell_height.max(1),
            },
            color: Color([220, 223, 228, 255]),
        });
        stats.quads = self
            .rows
            .iter()
            .map(|row| row.solids.len() + row.glyphs.len())
            .sum();
        stats
    }
}

#[allow(clippy::too_many_arguments)]
fn build_row<S: GlyphSource>(
    row: &mut Row,
    screen: &Screen,
    y: u16,
    x0: u16,
    cols: u16,
    metrics: Metrics,
    atlas: &mut Atlas,
    source: &mut S,
    stats: &mut Stats,
) {
    row.solids.clear();
    row.glyphs.clear();
    let end = x0.saturating_add(cols).min(screen.cols);
    for x in x0..end {
        let Some(cell) = screen.cell(x, y) else {
            continue;
        };
        let rect = Rect {
            x: x.saturating_mul(metrics.cell_width),
            y: y.saturating_mul(metrics.cell_height),
            w: metrics.cell_width,
            h: metrics.cell_height,
        };
        if let Some(bg) = cell.style.bg {
            row.solids.push(Solid {
                rect,
                color: color(bg),
            });
        }
        if cell.text.is_empty() {
            continue;
        }
        let key = GlyphKey {
            text: cell.text.clone(),
            bold: cell.style.bold,
            italic: cell.style.italic,
        };
        let size = metrics.cell_height.max(1);
        let slot = if let Some(slot) = atlas.get(&key, size) {
            Some(slot)
        } else {
            match source.glyph(&key, size) {
                Some(glyph) => {
                    stats.glyphs_rasterized += 1;
                    atlas.insert(key.clone(), size, &glyph)
                }
                None => None,
            }
        };
        if let Some(slot) = slot {
            row.glyphs.push(GlyphDraw {
                rect: Rect {
                    x: rect.x,
                    y: rect.y + metrics.baseline.min(metrics.cell_height),
                    w: slot.advance.max(1),
                    h: slot.uv.h.max(1),
                },
                uv: slot.uv,
                color: color(
                    cell.style
                        .fg
                        .unwrap_or(crate::StyleColor::Rgb([220, 223, 228])),
                ),
                key: key.clone(),
                pixel_size: size,
            });
        }
        if cell.style.underline != 0 || cell.style.strikethrough {
            let line = Rect {
                x: rect.x,
                y: rect.y
                    + if cell.style.strikethrough {
                        metrics.cell_height / 2
                    } else {
                        metrics.cell_height.saturating_sub(2)
                    },
                w: rect.w,
                h: 1,
            };
            row.solids.push(Solid {
                rect: line,
                color: color(
                    cell.style
                        .underline_color
                        .or(cell.style.fg)
                        .unwrap_or(crate::StyleColor::Rgb([220, 223, 228])),
                ),
            });
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u16,
    pub height: u16,
    pub pixels: Vec<u8>,
}

impl Image {
    /// Stable FNV-1a over dimensions and RGBA, for golden regressions.
    pub fn checksum(&self) -> u64 {
        let mut hash = 0xcbf29ce484222325u64;
        for byte in self
            .width
            .to_le_bytes()
            .into_iter()
            .chain(self.height.to_le_bytes())
            .chain(self.pixels.iter().copied())
        {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash
    }

    pub fn pixel(&self, x: u16, y: u16) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = (y as usize * self.width as usize + x as usize) * 4;
        Some([
            self.pixels[i],
            self.pixels[i + 1],
            self.pixels[i + 2],
            self.pixels[i + 3],
        ])
    }

    fn fill(&mut self, rect: Rect, color: Color) {
        for y in rect.y..rect.y.saturating_add(rect.h).min(self.height) {
            for x in rect.x..rect.x.saturating_add(rect.w).min(self.width) {
                let i = (y as usize * self.width as usize + x as usize) * 4;
                self.pixels[i..i + 4].copy_from_slice(&color.0);
            }
        }
    }

    fn blend_glyph(&mut self, glyph: &GlyphDraw, coverage: &[u8]) {
        let w = glyph.uv.w as usize;
        let h = glyph.uv.h as usize;
        for cy in 0..h {
            for cx in 0..w {
                let Some(alpha) = coverage.get(cy * w + cx).copied() else {
                    continue;
                };
                if alpha == 0 {
                    continue;
                }
                let x = glyph.rect.x.saturating_add(cx as u16);
                let y = glyph.rect.y.saturating_add(cy as u16);
                if x >= self.width || y >= self.height {
                    continue;
                }
                let i = (y as usize * self.width as usize + x as usize) * 4;
                for channel in 0..3 {
                    let src = u16::from(glyph.color.0[channel]);
                    let dst = u16::from(self.pixels[i + channel]);
                    self.pixels[i + channel] =
                        ((src * u16::from(alpha) + dst * (255 - u16::from(alpha))) / 255) as u8;
                }
            }
        }
    }
}

fn scale(value: u16, percent: u16) -> u16 {
    let scaled = (u32::from(value) * u32::from(percent) + 50) / 100;
    scaled.clamp(1, u32::from(u16::MAX)) as u16
}

fn color(style: crate::StyleColor) -> Color {
    match style {
        crate::StyleColor::Rgb([r, g, b]) => Color([r, g, b, 255]),
        crate::StyleColor::Palette(index) => Color([index, index, index, 255]),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{Screen, Vt};

    #[derive(Default)]
    struct Blocks {
        calls: usize,
        sizes: BTreeMap<(String, u16), usize>,
    }

    impl GlyphSource for Blocks {
        fn glyph(&mut self, key: &GlyphKey, size: u16) -> Option<Glyph> {
            self.calls += 1;
            *self.sizes.entry((key.text.clone(), size)).or_default() += 1;
            Some(Glyph {
                width: 1,
                height: size,
                advance: size,
                coverage: vec![255; size as usize],
            })
        }
    }

    fn metrics() -> Metrics {
        Metrics {
            cell_width: 8,
            cell_height: 16,
            baseline: 3,
        }
    }

    fn make_screen(bytes: &[u8]) -> Screen {
        let mut vt = Vt::new(4, 2);
        vt.feed(bytes);
        vt.screen()
    }

    #[test]
    fn only_damaged_rows_rebuild_and_cells_are_cached_across_densities() {
        let mut source = Blocks::default();
        let mut renderer = Renderer::new(metrics(), 4096, 64);
        let mut screen = make_screen(b"abcd\r\nefgh");
        renderer.update(&screen, &[], &mut source);
        assert_eq!(source.calls, 8, "one raster per distinct cell text");

        screen = make_screen(b"abcd\r\nefXh");
        let stats = renderer.update(
            &screen,
            &[Damage {
                x: 2,
                y: 1,
                cols: 1,
                rows: 1,
            }],
            &mut source,
        );
        assert_eq!(stats.rows_rebuilt, 1);
        assert_eq!(source.calls, 9, "only the new glyph was rasterized");

        let mut dense = Renderer::new(
            Metrics {
                cell_width: 16,
                cell_height: 32,
                baseline: 6,
            },
            8192,
            64,
        );
        dense.update(&screen, &[], &mut source);
        assert_eq!(
            source.sizes.get(&("X".into(), 32)).copied(),
            Some(1),
            "a new density gets its own atlas slot"
        );
    }

    #[test]
    fn hidden_panes_do_no_glyph_or_batch_work() {
        let mut source = Blocks::default();
        let mut renderer = Renderer::new(metrics(), 4096, 64);
        renderer.set_visible(false);
        let stats = renderer.update(&make_screen(b"hidden"), &[], &mut source);
        assert!(stats.hidden);
        assert_eq!(source.calls, 0);
        assert_eq!(stats.rows_rebuilt, 0);
        renderer.set_visible(true);
        assert_eq!(
            renderer
                .update(&make_screen(b"hidden"), &[], &mut source)
                .rows_rebuilt,
            2,
            "showing a pane forces one full rebuild"
        );
    }

    #[test]
    fn the_atlas_is_bounded_and_survives_gl_recreation() {
        let mut source = Blocks::default();
        let mut renderer = Renderer::new(metrics(), 64, 64);
        renderer.update(&make_screen(b"abcdefgh"), &[], &mut source);
        assert!(renderer.atlas().bytes() <= 64);
        assert!(renderer.atlas().bytes() > 0, "some glyphs fit");
        let before = renderer.atlas().bytes();
        let generation = renderer.recreate_gl();
        assert_eq!(generation, 1);
        assert_eq!(renderer.atlas().bytes(), before, "CPU atlas is retained");
    }

    #[test]
    fn software_composition_draws_real_glyphs_and_scales_with_density() {
        let background = Color([0, 0, 0, 255]);
        let mut font = crate::DejaVuFont::new();
        let screen = make_screen(b"A");
        let mut renderer = Renderer::new(metrics(), 64 * 1024, 128);
        renderer.update(&screen, &[], &mut font);
        let image = renderer.compose(background);
        assert_eq!((image.width, image.height), (32, 32));
        assert!(
            image
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| *pixel != background.0),
            "the glyph must leave visible coverage"
        );

        let mut dense = Renderer::new(
            Metrics {
                cell_width: 16,
                cell_height: 32,
                baseline: 6,
            },
            64 * 1024,
            128,
        );
        dense.update(&screen, &[], &mut font);
        let bigger = dense.compose(background);
        assert_eq!((bigger.width, bigger.height), (64, 64));
        assert_ne!(image.pixels, bigger.pixels);
    }

    #[test]
    fn software_goldens_are_stable_across_densities_and_backgrounds() {
        let mut font = crate::DejaVuFont::new();
        let screen = make_screen(b"A");
        let mut standard = Renderer::new(metrics(), 64 * 1024, 128);
        standard.update(&screen, &[], &mut font);
        let black = standard.compose(Color([0, 0, 0, 255]));
        let white = standard.compose(Color([255, 255, 255, 255]));

        let mut dense = Renderer::new(
            Metrics {
                cell_width: 16,
                cell_height: 32,
                baseline: 6,
            },
            64 * 1024,
            128,
        );
        dense.update(&screen, &[], &mut font);
        let large = dense.compose(Color([0, 0, 0, 255]));

        assert_eq!(black.checksum(), 0xda45f5563ad4140d, "1x black golden");
        assert_eq!(white.checksum(), 0x74a3ae5f482f967c, "1x white golden");
        assert_eq!(large.checksum(), 0x508af6343fcc6c9f, "2x black golden");
    }

    #[test]
    fn zoom_is_bounded_and_rescales_geometry_and_glyphs() {
        let mut source = Blocks::default();
        let mut renderer = Renderer::new(metrics(), 64 * 1024, 128);
        assert_eq!(renderer.zoom(), 100);
        assert_eq!(renderer.set_zoom(10), MIN_ZOOM_PERCENT);
        assert_eq!(renderer.metrics().cell_height, 8);
        assert_eq!(renderer.set_zoom(10_000), MAX_ZOOM_PERCENT);
        assert_eq!(renderer.metrics().cell_height, 48);

        let screen = make_screen(b"A");
        renderer.update(&screen, &[], &mut source);
        assert!(
            source.sizes.contains_key(&("A".into(), 48)),
            "zoomed glyphs get their own atlas density"
        );
        let image = renderer.compose(Color([0, 0, 0, 255]));
        assert_eq!((image.width, image.height), (96, 96));

        assert_eq!(renderer.set_zoom(150), 150);
        assert_eq!(renderer.metrics().cell_width, 12);
    }

    #[test]
    fn cursor_is_a_separate_quad() {
        let mut source = Blocks::default();
        let mut renderer = Renderer::new(metrics(), 4096, 64);
        renderer.update(&make_screen(b"\x1b[2;2H"), &[], &mut source);
        let cursor = renderer.cursor().expect("visible cursor");
        assert_eq!((cursor.rect.x, cursor.rect.y), (8, 16));
    }
}
