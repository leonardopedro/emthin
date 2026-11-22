//! Glyph coverage from the same DejaVu face bundled with velysterm.
//!
//! A cell may hold a full grapheme cluster. Each scalar is laid out left to
//! right into one coverage bitmap; shaping beyond that (ligatures, bidi,
//! mark positioning) stays with the platform font stack.

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};

use crate::{Glyph, GlyphKey, GlyphSource};

const FONT: &[u8] = include_bytes!("../../../../velysterm/assets/fonts/dejavu.ttf");

pub struct DejaVuFont {
    font: FontRef<'static>,
}

impl Default for DejaVuFont {
    fn default() -> Self {
        Self {
            font: FontRef::try_from_slice(FONT).expect("bundled DejaVu is a font"),
        }
    }
}

impl DejaVuFont {
    pub fn new() -> Self {
        Self::default()
    }
}

impl GlyphSource for DejaVuFont {
    fn glyph(&mut self, key: &GlyphKey, pixel_size: u16) -> Option<Glyph> {
        let scale = PxScale::from(f32::from(pixel_size.max(1)));
        let scaled = self.font.as_scaled(scale);
        let mut glyphs = Vec::new();
        let mut width = 0u16;
        let mut height = 0u16;
        for ch in key.text.chars() {
            let id = self.font.glyph_id(ch);
            let advance = scaled.h_advance(id).ceil().max(0.0) as u16;
            let x0 = width;
            let glyph =
                id.with_scale_and_position(scale, ab_glyph::point(x0 as f32, scaled.ascent()));
            if let Some(outline) = self.font.outline_glyph(glyph) {
                let bounds = outline.px_bounds();
                height = height.max(bounds.max.y.ceil().max(0.0) as u16);
                glyphs.push((outline, x0));
            }
            width = width.saturating_add(advance);
        }
        let height = height.max(1);
        let width = width.max(1);
        let mut coverage = vec![0u8; width as usize * height as usize];
        for (outline, x0) in glyphs {
            outline.draw(|x, y, c| {
                let px = x0.saturating_add(x as u16);
                let py = y as u16;
                if px < width && py < height {
                    let index = py as usize * width as usize + px as usize;
                    coverage[index] = coverage[index].max((c.clamp(0.0, 1.0) * 255.0) as u8);
                }
            });
        }
        Some(Glyph {
            width,
            height,
            advance: width,
            coverage,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_visible_glyph_has_coverage_and_a_space_does_not() {
        let mut font = DejaVuFont::new();
        let a = font
            .glyph(
                &GlyphKey {
                    text: "A".into(),
                    bold: false,
                    italic: false,
                },
                16,
            )
            .expect("glyph");
        assert!(a.width > 0 && a.height > 0);
        assert!(a.coverage.iter().any(|value| *value > 0));
        let space = font
            .glyph(
                &GlyphKey {
                    text: " ".into(),
                    bold: false,
                    italic: false,
                },
                16,
            )
            .expect("space advance");
        assert!(space.coverage.iter().all(|value| *value == 0));
    }

    #[test]
    fn density_changes_the_bitmap_size() {
        let mut font = DejaVuFont::new();
        let small = font
            .glyph(
                &GlyphKey {
                    text: "A".into(),
                    bold: false,
                    italic: false,
                },
                16,
            )
            .unwrap();
        let large = font
            .glyph(
                &GlyphKey {
                    text: "A".into(),
                    bold: false,
                    italic: false,
                },
                32,
            )
            .unwrap();
        assert!(large.width > small.width && large.height > small.height);
    }
}
