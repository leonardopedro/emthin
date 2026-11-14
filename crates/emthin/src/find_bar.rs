//! The find bar's one-line label: query, status, and the close hint.
//!
//! The search itself lives in `docui::search` and is shared with the pad.
//! This module owns only the pixels: the bar is a small rasterized panel
//! because the compositor's overlays are textures/flat colours, not a text
//! widget toolkit. It is deliberately the same path as the dormant-figure
//! label, so the find bar uses the document's own font and needs no second
//! text renderer.
//!
//! Layout is cached by the visible string. Typing one character re-rasterizes
//! once; the render loop can ask every frame without paying Typst's layout cost
//! per frame.

use crate::dormant_label::{element, LabelImage};

/// Find-bar images keyed by the exact text that produced them.
#[derive(Default)]
pub struct FindBarCache {
    key: Option<String>,
    image: Option<LabelImage>,
}

impl FindBarCache {
    /// The raster for `query` + `status`, or `None` when it cannot render.
    ///
    /// `status` is `""` for an empty query, `n/m` for a match, or
    /// `no match`; it is supplied by `FindBar::status`.
    pub fn get(&mut self, query: &str, status: &str) -> Option<&LabelImage> {
        let key = label_text(query, status);
        if self.key.as_deref() != Some(key.as_str()) {
            self.key = Some(key);
            self.image = rasterize(query, status);
        }
        self.image.as_ref()
    }
}

/// The literal line shown in the bar.
///
/// Kept separate from the markup so the cache key and the displayed text
/// cannot drift apart, and so the empty-query hint is testable without a
/// renderer.
pub(crate) fn label_text(query: &str, status: &str) -> String {
    if query.is_empty() {
        return "Find  Esc close".to_owned();
    }
    if status.is_empty() {
        format!("Find: {query}")
    } else {
        format!("Find: {query}  {status}")
    }
}

/// Rasterize the bar through mathed_mini, escaping the typed query.
fn rasterize(query: &str, status: &str) -> Option<LabelImage> {
    // The query is arbitrary typed text. Render it as `raw` so a `#` or `*`
    // cannot become markup, exactly like a figure's launch name.
    let style = "#set text(fill: white, size: 18pt, font: \"DejaVu Sans Mono\")\n";
    let markup = if query.is_empty() {
        format!(
            "{style}#text(fill: rgb(\"#8ecdf2\"))[Find] \
             #text(fill: rgb(\"#7f8c9a\"))[  Esc close]"
        )
    } else {
        let escaped = query.replace('\\', "\\\\").replace('"', "\\\"");
        let mut markup = format!("{style}#text(fill: rgb(\"#8ecdf2\"))[Find] #raw(\"");
        markup.push_str(&escaped);
        markup.push_str("\")");
        if !status.is_empty() {
            let fill = if status == "no match" {
                "#ff8a80"
            } else {
                "#b8c7d9"
            };
            markup.push_str(&format!(" #text(fill: rgb(\"{fill}\"))[{status}]"));
        }
        markup
    };
    // Render raw Typst, not the document pipeline: the bar has its own
    // `#text(...)` styling, and `rasterize_snippet_raw` runs mathed's marker
    // transform, which would print the function names literally.
    let image = mathed_mini::render::render_markup(&markup, 560.0).ok()?;
    Some(LabelImage {
        width: image.width,
        height: image.height,
        rgba: image.data,
    })
}

/// Build the GPU element for a cached find-bar image at logical `x, y`.
///
/// The bar is UI, not page content, so it stays at native output scale rather
/// than following the page's letterbox factor.
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
    fn the_empty_query_shows_the_close_hint() {
        assert_eq!(label_text("", ""), "Find  Esc close");
    }

    #[test]
    fn a_match_reports_its_position_and_a_miss_reports_the_reason() {
        assert_eq!(label_text("alpha", "2/3"), "Find: alpha  2/3");
        assert_eq!(label_text("zi", "no match"), "Find: zi  no match");
    }

    #[test]
    fn markup_in_the_query_stays_literal() {
        for query in ["#emph(x)", "*stars*", r#"a "quote" \ b"#] {
            let image =
                rasterize(query, "1/1").unwrap_or_else(|| panic!("{query:?} must rasterize"));
            assert!(image.width > 0 && image.height > 0, "{query:?}");
        }
    }

    #[test]
    fn a_query_with_repeated_spaces_keeps_them() {
        assert_eq!(label_text("a  b", "1/1"), "Find: a  b  1/1");
        assert!(rasterize("a  b", "1/1").is_some());
    }

    #[test]
    fn the_cache_replaces_the_image_when_the_text_changes() {
        let mut cache = FindBarCache::default();
        let first = cache.get("a", "1/2").expect("first").width;
        let second = cache.get("ab", "1/1").expect("second").width;
        assert!(
            second > first,
            "the query grew, so the raster should too ({first} -> {second})"
        );
        assert_eq!(
            cache.key.as_deref(),
            Some("Find: ab  1/1"),
            "the cache keeps the newest text"
        );
    }
}
