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
            element::{texture::TextureRenderElement, Id, Kind},
            gles::{GlesRenderer, GlesTexture},
            ContextId, ImportMem, Renderer,
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
    texture: Option<GlesTexture>,
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
        self.texture = None;
        self.key = None;
    }

    /// True when `pixels` would require a fresh upload.
    pub fn is_stale(&self, page: usize, revision: u64) -> bool {
        self.key != Some((page, revision))
    }

    /// Import the page's pixels if they differ from what's cached, and
    /// return the render element placing it at `rect`.
    ///
    /// `import_memory` keeps a reference to the slice, so the buffer has
    /// to outlive the texture. The bytes are therefore leaked per
    /// re-import: pages re-import on *edit*, not on pointer motion, so
    /// this is bounded by typing speed rather than frame rate, and the
    /// alternative (a persistent staging buffer with explicit fences) is
    /// a lot of machinery to avoid a few hundred MB over a long session.
    pub fn element(
        &mut self,
        renderer: &mut GlesRenderer,
        pixels: &PagePixels<'_>,
    ) -> Option<CustomElement<GlesRenderer>> {
        let key = (pixels.page, pixels.revision);
        if self.key != Some(key) {
            let owned: &'static [u8] = Box::leak(pixels.rgba.to_vec().into_boxed_slice());
            let size = Size::new(pixels.width as i32, pixels.height as i32);
            match renderer.import_memory(owned, Fourcc::Rgba8888, size, false) {
                Ok(texture) => {
                    self.texture = Some(texture);
                    self.key = Some(key);
                }
                Err(e) => {
                    tracing::warn!("doc page import failed: {e:?}");
                    // Keep the old texture: a blank page is worse than a
                    // stale one.
                }
            }
        }
        let texture = self.texture.as_ref()?;
        let ctx: ContextId<GlesTexture> = renderer.context_id();
        // Namespaced per page so the damage tracker doesn't collapse two
        // pages' damage into one region.
        let id = Id::new().namespaced(pixels.page.wrapping_add(1));
        // `size` is in *logical* px (the texture is sampled 1:1 with the
        // raster); only `location` is physical, because the element
        // itself is placed in output coordinates.
        Some(
            TextureRenderElement::from_static_texture(
                id,
                ctx,
                pixels.rect.loc.to_f64().to_physical(pixels.scale),
                texture.clone(),
                1,
                Transform::Normal,
                None,
                None,
                Some(pixels.rect.size),
                None,
                Kind::Unspecified,
            )
            .into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_cache_is_stale_for_every_page() {
        let cache = DocPageTexture::new();
        assert!(cache.is_stale(0, 0));
        assert!(cache.texture.is_none());
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
}
