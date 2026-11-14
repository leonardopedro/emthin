//! The math completion preview at the caret.
//!
//! `DocUi` owns the pending [`mathed_core::completion::Completion`]; this module
//! owns only its pixels. Like the find bar and dormant-figure labels, the glyph
//! is a small raster because the compositor's overlays are textures and flat
//! colours, not a text widget toolkit. It goes through mathed_mini so the hint
//! uses the document's own font and needs no second text renderer.
//!
//! The preview is keyed by the glyph it shows. Editing inside one run can change
//! the pending replacement without changing the glyph (`\al` and `\alpha` both
//! preview `α`), so the raster then correctly stays cached.

use crate::dormant_label::{element, LabelImage};

/// Completion-preview images keyed by the glyph they display.
#[derive(Default)]
pub struct CompletionHintCache {
    key: Option<String>,
    image: Option<LabelImage>,
}

impl CompletionHintCache {
    /// The raster for `glyph`, or `None` when it cannot render.
    pub fn get(&mut self, glyph: &str) -> Option<&LabelImage> {
        if self.key.as_deref() != Some(glyph) {
            self.key = Some(glyph.to_owned());
            self.image = rasterize(glyph);
        }
        self.image.as_ref()
    }
}

/// Rasterize the completion glyph through mathed_mini.
fn rasterize(glyph: &str) -> Option<LabelImage> {
    let style = "#set text(fill: rgb(\"#8ecdf2\"), size: 16pt, font: \"DejaVu Sans Mono\")\n";
    let escaped = glyph.replace('\\', "\\\\").replace('"', "\\\"");
    let markup = format!("{style}#raw(\"{escaped}\")");
    let image = mathed_mini::render::render_markup(&markup, 160.0).ok()?;
    Some(LabelImage {
        width: image.width,
        height: image.height,
        rgba: image.data,
    })
}

/// Build the GPU element for a cached completion preview at logical `x, y`.
pub fn draw(
    renderer: &mut smithay::backend::renderer::gles::GlesRenderer,
    image: &LabelImage,
    x: i32,
    y: i32,
    scale: f64,
) -> Option<crate::element::CustomElement<smithay::backend::renderer::gles::GlesRenderer>> {
    element(renderer, image, x, y, scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_math_glyph_rasterizes() {
        for glyph in ["→", "α", "≤", "⟶"] {
            let image = rasterize(glyph).unwrap_or_else(|| panic!("{glyph:?} must rasterize"));
            assert!(image.width > 0 && image.height > 0, "{glyph:?}");
        }
    }

    #[test]
    fn the_cache_replaces_the_image_when_the_glyph_changes() {
        let mut cache = CompletionHintCache::default();
        cache.get("→").expect("arrow");
        assert_eq!(cache.key.as_deref(), Some("→"));
        cache.get("α").expect("alpha");
        assert_eq!(
            cache.key.as_deref(),
            Some("α"),
            "the cache tracks the glyph"
        );
    }
}
