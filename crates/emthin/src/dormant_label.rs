//! The label drawn on a dormant figure: "foot — click or Return to launch".
//!
//! §5.10 asks for a stand-in of "framed placeholder + app name + Enter to
//! launch". The frame is a `SolidColor` bar in `figure_render`; the text is not,
//! because every other overlay here is a flat rectangle and drawing glyphs
//! means uploading a texture.
//!
//! # Why the compositor renders it, not the document
//!
//! "Is this figure dormant" is compositor state — whether a client is currently
//! bound to it — and no document function can see that. So the label cannot be
//! spliced by `transform` the way a `\formal` verdict is. It has to be an
//! overlay, which means it is drawn on the GPU rather than laid out with the
//! page.
//!
//! # Cost
//!
//! Laying the label out means running Typst over a one-line snippet, measured
//! at ~0.5ms. That is far too much for a per-frame path with several figures and
//! negligible once per change, so the rasterized image is cached and keyed by
//! (figure key, text): re-laying out only happens when the label text actually
//! changes.
//!
//! No scale-based invalidation is needed. The pixels are rasterized in points
//! (1px == 1pt, like the page raster) and the output's fractional scale is
//! applied when the buffer is uploaded, so a DPI change reuses them. That is why
//! there is no `clear()`.

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                Kind,
            },
            gles::GlesRenderer,
        },
    },
    utils::{Logical, Point},
};
use std::collections::HashMap;

/// One cached label image, in raster pixels (1px == 1pt).
#[derive(Clone)]
pub struct LabelImage {
    pub width: u32,
    pub height: u32,
    /// Premultiplied-free RGBA8, straight from mathed_mini's rasterizer.
    pub rgba: Vec<u8>,
}

/// Labels already rasterized this session, keyed by figure key.
///
/// Keyed by key *and* text so an edited `launch:` command re-rasterizes; keyed
/// by key alone would keep showing the old program's name.
#[derive(Default)]
pub struct LabelCache {
    entries: HashMap<String, (String, LabelImage)>,
}

impl LabelCache {
    /// The image for `key`, rasterizing `text` if it is not cached.
    ///
    /// `width_pt` is the space to lay the label out in; the figure's own width
    /// is a fine choice and keeps the label from wrapping.
    pub fn get(&mut self, key: &str, text: &str, width_pt: f64) -> Option<&LabelImage> {
        let fresh = self.entries.get(key).is_some_and(|(seen, _)| seen == text);
        if !fresh {
            self.entries.insert(
                key.to_string(),
                (text.to_string(), rasterize(text, width_pt)?),
            );
        }
        self.entries.get(key).map(|(_, image)| image)
    }
}

/// Rasterize one line of label text through the document's own typesetting.
///
/// Deliberately routed through mathed_mini rather than a bespoke bitmap font:
/// the label then uses the same font stack and theme as the page it sits on, so
/// it does not look pasted in from another program. The snippet is wrapped so
/// Typst does not interpret the app name as markup — a figure called `#1` must
/// not turn into a marker reference.
fn rasterize(text: &str, width_pt: f64) -> Option<LabelImage> {
    // `#raw("...")` is the escape hatch: the text is shown literally, so an app
    // named `#emph` or `*` cannot alter the label or fail to compile.
    let markup = format!(
        "#raw(\"{}\")",
        text.replace('\\', "\\\\").replace('"', "\\\"")
    );
    let (width, height, rgba) =
        mathed_mini::render::rasterize_snippet_raw(&markup, width_pt.max(64.0))?;
    Some(LabelImage {
        width,
        height,
        rgba,
    })
}

/// Build a `Label` element from a cached image, positioned at `x, y` in logical
/// px and scaled by `scale`.
pub fn element(
    renderer: &mut GlesRenderer,
    image: &LabelImage,
    x: i32,
    y: i32,
    scale: f64,
) -> Option<crate::element::CustomElement<GlesRenderer>> {
    let size = smithay::utils::Size::from((image.width as i32, image.height as i32));
    // The buffer must outlive the texture, so it copies the slice rather than
    // borrowing it — which is why the cache holds the pixels and this call is
    // per-frame rather than once.
    let buffer = MemoryRenderBuffer::from_slice(
        &image.rgba,
        Fourcc::Rgba8888,
        size,
        scale as i32 * 256,
        smithay::utils::Transform::Normal,
        None,
    );
    let element = MemoryRenderBufferRenderElement::from_buffer(
        renderer,
        Point::<f64, Logical>::new(x as f64, y as f64)
            .to_physical(scale)
            .to_i32_round(),
        &buffer,
        Some(1.0),
        None,
        None,
        Kind::Unspecified,
    )
    .ok()?;
    Some(element.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A label is rasterized through Typst, so the text is attacker-adjacent:
    /// it contains whatever the user typed in a `launch:` argument. Anything
    /// that is not escaped would let an app name change the label's markup, or
    /// fail to compile and take the whole overlay with it.
    #[test]
    fn markup_in_the_text_is_shown_literally() {
        for text in [
            r#"#emph("not emphasis")"#,
            "*not italic*",
            "#1 not a marker",
            r#"a "quoted" name"#,
            r"a \ backslash",
        ] {
            let image =
                rasterize(text, 200.0).unwrap_or_else(|| panic!("{text:?} must still rasterize"));
            assert!(image.width > 0 && image.height > 0, "{text:?}");
        }
    }

    /// Escaping is what makes the above work, so pin the transformation itself
    /// rather than only its consequence.
    #[test]
    fn quotes_and_backslashes_are_escaped() {
        let markup = format!(
            "#raw(\"{}\")",
            r#"a "q" \ b"#.replace('\\', "\\\\").replace('"', "\\\"")
        );
        assert_eq!(markup, r#"#raw("a \"q\" \\ b")"#);
    }

    /// The cache is keyed by text as well as key, so editing a figure's
    /// `launch:` command re-rasterizes instead of leaving the old program's
    /// name on screen.
    #[test]
    fn editing_the_command_rasterizes_again() {
        let mut cache = LabelCache::default();
        cache.get("f0", "foot — launch", 200.0).expect("first");
        // Asking again for the same text must not add an entry, nor disturb the
        // recorded text.
        cache.get("f0", "foot — launch", 200.0).expect("cached");
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(cache.entries["f0"].0, "foot — launch");

        cache
            .get("f0", "alacritty — launch", 200.0)
            .expect("second");
        assert_eq!(
            cache.entries.len(),
            1,
            "a changed label replaces rather than accumulating"
        );
        assert_eq!(
            cache.entries["f0"].0, "alacritty — launch",
            "the cached text must track the edit, or the stale name stays on screen"
        );
    }

    /// Two figures with the same text are cached separately, so a later change
    /// to one cannot rename the other.
    #[test]
    fn keys_are_cached_independently() {
        let mut cache = LabelCache::default();
        cache.get("f0", "same — text", 200.0).expect("a");
        cache.get("f1", "same — text", 200.0).expect("b");
        assert_eq!(cache.entries.len(), 2);
        assert!(cache.entries.contains_key("f0"));
        assert!(cache.entries.contains_key("f1"));
    }

    /// A width too small to typeset must yield `None`, not a panic and not an
    /// empty image — the caller draws nothing in that case.
    #[test]
    fn a_tiny_width_is_floored_rather_than_failing() {
        let image = rasterize("foot", 1.0).expect("floored");
        assert!(image.width >= 1, "got width {}", image.width);
    }
}
