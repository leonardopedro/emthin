//! The bottom status line's raster cache.
//!
//! Same path as the find bar and dormant-figure labels: a small Typst raster
//! through mathed_mini, cached by the exact visible string. The compositor's
//! overlays are textures, not a text widget toolkit, so this is the whole UI.

use crate::dormant_label::{element, LabelImage};

/// Status images keyed by the exact line that produced them.
#[derive(Default)]
pub struct StatusBarCache {
    key: Option<String>,
    image: Option<LabelImage>,
}

impl StatusBarCache {
    pub fn get(&mut self, text: &str) -> Option<&LabelImage> {
        if self.key.as_deref() != Some(text) {
            self.key = Some(text.to_owned());
            self.image = rasterize(text);
        }
        self.image.as_ref()
    }
}

fn rasterize(text: &str) -> Option<LabelImage> {
    let style = "#set text(fill: rgb(\"#d8e3ee\"), size: 16pt, font: \"DejaVu Sans Mono\")\n";
    let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
    let markup = format!("{style}#raw(\"{escaped}\")");
    let image = mathed_mini::render::render_markup(&markup, 960.0).ok()?;
    Some(LabelImage {
        width: image.width,
        height: image.height,
        rgba: image.data,
    })
}

/// Build the GPU element for a cached status image.
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
    fn markup_in_the_status_stays_literal() {
        for text in ["#emph(x)", "*stars*", r#"a "quote" \ b"#] {
            let image = rasterize(text).unwrap_or_else(|| panic!("{text:?} must rasterize"));
            assert!(image.width > 0 && image.height > 0, "{text:?}");
        }
    }

    #[test]
    fn the_cache_replaces_the_image_when_the_line_changes() {
        let mut cache = StatusBarCache::default();
        let first = cache.get("paper.typ  unsaved").expect("first").width;
        let second = cache
            .get("paper.typ  unsaved  Find 2/3")
            .expect("second")
            .width;
        assert!(second > first, "{first} -> {second}");
    }
}
