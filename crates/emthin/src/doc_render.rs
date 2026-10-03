//! The document page as a GPU texture.
//!
//! `mathed_mini` rasterizes the page on the CPU (1px == 1pt, white text
//! on transparent — see its `THEME_PRELUDE`); this module owns that
//! buffer's trip to the GPU and its invalidation.
//!
//! Re-uploading a full A4 page every frame would be ~8 MB/frame of PCIe
//! traffic for a mostly-static document, so the texture is cached and
//! only re-imported when the page's pixels actually change (a layout
//! pass, a page switch, or a caret-reveal toggle that re-typesets a
//! statement).

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
    utils::{Logical, Rectangle, Size, Transform},
};

use crate::element::CustomElement;

/// The current page's raster, cached on the GPU.
///
/// `Key` is everything that can change the pixels: the page index (so a
/// page switch re-imports even if the layout didn't change) and the
/// document revision (so any edit re-imports).
#[derive(Default)]
pub struct DocPageTexture {
    /// The page's pixels, owned. `MemoryRenderBuffer` copies the slice into an
    /// `Arc`, and `import_texture` caches the GL texture per context and uploads
    /// only the damaged region, so keeping the buffer here costs one copy per
    /// *document revision* and nothing per frame.
    buffer: Option<MemoryRenderBuffer>,
    /// The width the current buffer was built at, so a resize is visible without
    /// reaching into the buffer type. Only the tests read it.
    #[cfg_attr(not(test), allow(dead_code))]
    width: usize,
    key: Option<(usize, u64)>,
}

/// Everything [`DocPageTexture::element`] needs about the current page.
pub struct PagePixels<'a> {
    /// Tightly packed RGBA8, row-major.
    pub rgba: &'a [u8],
    pub width: u32,
    pub height: u32,
    /// 0-based page index.
    pub page: usize,
    /// Document revision — bumped by any edit.
    pub revision: u64,
    /// Where the page sits in output-local logical px.
    pub rect: Rectangle<i32, Logical>,
    /// Output scale (for fractional DPI).
    pub scale: f64,
}

impl DocPageTexture {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop the cached texture. Called when the GL context is lost or
    /// rebuilt (a resize that re-creates the winit surface).
    pub fn clear(&mut self) {
        self.buffer = None;
        self.key = None;
    }

    /// True when `pixels` would require a fresh upload.
    pub fn is_stale(&self, page: usize, revision: u64) -> bool {
        self.key != Some((page, revision))
    }

    /// Copy `pixels` into an owned buffer, replacing any previous one.
    ///
    /// Split out of [`Self::element`] so the ownership claim is testable without
    /// a GL context: the point is that the previous buffer is *dropped* here,
    /// which is what a leaked `&'static [u8]` could never be checked for.
    fn adopt(&mut self, pixels: &PagePixels<'_>) {
        let size = Size::new(pixels.width as i32, pixels.height as i32);
        // `from_slice` copies into an `Arc`, so the bytes are owned and are
        // released when the previous buffer is dropped.
        self.buffer = Some(MemoryRenderBuffer::from_slice(
            pixels.rgba,
            Fourcc::Rgba8888,
            size,
            // Buffer scale and transform describe the pixels themselves, not
            // where they land on screen; the element's `size` places them.
            256,
            Transform::Normal,
            None,
        ));
        self.width = pixels.width as usize;
        self.key = Some((pixels.page, pixels.revision));
    }

    /// How many page buffers this cache is holding. Always 0 or 1; a method so a
    /// test can say so rather than trusting the type.
    #[cfg(test)]
    fn buffer_count(&self) -> usize {
        usize::from(self.buffer.is_some())
    }

    /// The width the current buffer was built at. Kept alongside it so the
    /// ownership tests can check that a resize propagates without depending on
    /// `MemoryRenderBuffer::size`, which is not reachable in this build.
    #[cfg(test)]
    fn buffer_width(&self) -> usize {
        self.width
    }

    /// Import the page's pixels if they differ from what's cached, and
    /// return the render element placing it at `rect`.
    ///
    /// The pixels go through `MemoryRenderBuffer`, which **owns** its copy of
    /// the slice, so nothing is leaked.
    ///
    /// This used to be `Box::leak(pixels.rgba.to_vec())` per re-import, because
    /// `import_memory` borrows the slice for the texture's lifetime and wants a
    /// `&'static [u8]`. The re-import key is the document *revision*, which every
    /// keystroke bumps, and an A4 page at 1px/pt is 595x842x4 B — about 2 MB —
    /// so that leaked roughly 2 MB per character typed and the comment's estimate
    /// of "a few hundred MB over a long session" was out by orders of magnitude.
    ///
    /// The copy is not extra work: the old code did `to_vec()` too. It is just
    /// owned now instead of leaked.
    pub fn element(
        &mut self,
        renderer: &mut GlesRenderer,
        pixels: &PagePixels<'_>,
    ) -> Option<CustomElement<GlesRenderer>> {
        let key = (pixels.page, pixels.revision);
        if self.key != Some(key) {
            self.adopt(pixels);
        }

        let buffer = self.buffer.as_ref()?;
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            pixels.rect.loc.to_f64().to_physical(pixels.scale),
            buffer,
            Some(1.0),
            None,
            Some(pixels.rect.size),
            Kind::Unspecified,
        )
        .ok()
        .map(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_cache_is_stale_for_every_page() {
        let cache = DocPageTexture::new();
        assert!(cache.is_stale(0, 0));
        assert!(cache.buffer.is_none(), "nothing imported yet");
    }

    #[test]
    fn a_loaded_key_is_not_stale() {
        let mut cache = DocPageTexture::new();
        cache.key = Some((2, 7));
        assert!(
            !cache.is_stale(2, 7),
            "same page+revision reuses the texture"
        );
        assert!(cache.is_stale(2, 8), "a new revision re-imports");
        assert!(cache.is_stale(3, 7), "a page switch re-imports");
    }

    #[test]
    fn clearing_resets_the_key() {
        let mut cache = DocPageTexture::new();
        cache.key = Some((0, 1));
        cache.clear();
        assert!(cache.is_stale(0, 1), "a cleared cache must re-import");
    }

    /// The buffer is owned, so a re-import *replaces* it.
    ///
    /// This used to be `Box::leak(pixels.rgba.to_vec())` on every re-import,
    /// keyed by the document *revision* — which every keystroke bumps. An A4 page
    /// at 1px/pt is 595x842x4 B ≈ 2 MB, so that leaked about 2 MB per character
    /// typed, and the old comment's "a few hundred MB over a long session" was out
    /// by orders of magnitude.
    ///
    /// A leaked slice cannot be asserted on: nothing would ever report the old
    /// ones. Holding the buffer makes "exactly one exists" a fact a test can check.
    #[test]
    fn twenty_revisions_still_hold_exactly_one_buffer() {
        let mut cache = DocPageTexture::new();
        assert_eq!(cache.buffer_count(), 0, "nothing imported yet");

        let (w, h) = (8u32, 4u32);
        let bytes = vec![0xAAu8; (w * h * 4) as usize];

        for revision in 1..21u64 {
            let pixels = PagePixels {
                rgba: &bytes,
                width: w,
                height: h,
                page: 0,
                revision,
                rect: Rectangle::new((0, 0).into(), Size::from((w as i32, h as i32))),
                scale: 1.0,
            };
            cache.adopt(&pixels);
            assert_eq!(
                cache.buffer_count(),
                1,
                "revision {revision} must replace the buffer, not add one"
            );
        }
        assert_eq!(
            cache.key,
            Some((0, 20)),
            "the key tracks the newest revision"
        );
        assert_eq!(cache.buffer_width(), w as usize);
    }

    /// A resize changes the page dimensions, and the buffer must follow —
    /// otherwise the stale one is uploaded at the new size.
    #[test]
    fn a_resize_replaces_the_buffer_at_the_new_dimensions() {
        let mut cache = DocPageTexture::new();
        let small = vec![1u8; 4 * 4 * 4];
        let large = vec![1u8; 16 * 8 * 4];

        cache.adopt(&PagePixels {
            rgba: &small,
            width: 4,
            height: 4,
            page: 0,
            revision: 1,
            rect: Rectangle::new((0, 0).into(), Size::from((4, 4))),
            scale: 1.0,
        });
        assert_eq!(cache.buffer_width(), 4);

        cache.adopt(&PagePixels {
            rgba: &large,
            width: 16,
            height: 8,
            page: 0,
            revision: 2,
            rect: Rectangle::new((0, 0).into(), Size::from((16, 8))),
            scale: 1.0,
        });
        assert_eq!(cache.buffer_width(), 16, "width must follow the new page");
        assert_eq!(cache.buffer_count(), 1);
    }

    /// A page switch re-imports even at an unchanged revision, because page and
    /// revision are separate parts of the key.
    #[test]
    fn a_page_switch_is_a_distinct_key() {
        let mut cache = DocPageTexture::new();
        let bytes = vec![1u8; 4 * 4 * 4];
        let p = |page: usize| PagePixels {
            rgba: &bytes,
            width: 4,
            height: 4,
            page,
            revision: 7,
            rect: Rectangle::new((0, 0).into(), Size::from((4, 4))),
            scale: 1.0,
        };
        cache.adopt(&p(0));
        assert!(!cache.is_stale(0, 7));
        assert!(cache.is_stale(1, 7), "another page is another texture");
    }

    /// `clear` must drop the owned buffer, not just forget the key.
    #[test]
    fn clear_releases_the_buffer() {
        let mut cache = DocPageTexture::new();
        let bytes = vec![0u8; 4 * 4 * 4];
        cache.adopt(&PagePixels {
            rgba: &bytes,
            width: 4,
            height: 4,
            page: 0,
            revision: 1,
            rect: Rectangle::new((0, 0).into(), Size::from((4, 4))),
            scale: 1.0,
        });
        assert_eq!(cache.buffer_count(), 1);
        cache.clear();
        assert_eq!(cache.buffer_count(), 0, "the pixels must be released");
        assert!(cache.is_stale(0, 1), "and the key forgotten");
    }
}
